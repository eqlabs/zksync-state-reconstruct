use std::{future::Future, ops::Fn, sync::Arc};

use ethers::{
    abi::{Contract, Function},
    prelude::*,
    providers::Provider,
};
use eyre::Result;
use rand::random;
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    time::{sleep, Duration},
};

use crate::{
    constants::ethereum::{BLOCK_STEP, GENESIS_BLOCK, ZK_SYNC_ADDR},
    snapshot::StateSnapshot,
    types::{CommitBlockInfoV1, ParseError},
};

/// `MAX_RETRIES` is the maximum number of retries on failed L1 call.
const MAX_RETRIES: u8 = 5;
/// The interval in seconds in which to poll for new blocks.
const LONG_POLLING_INTERVAL_S: u64 = 120;
/// The interval in seconds to wait before retrying to fetch a previously failed transaction.
const FAILED_FETCH_RETRY_INTERVAL_S: u64 = 10;
/// The interval in seconds in which to print metrics.
const METRICS_PRINT_INTERVAL_S: u64 = 10;

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub enum L1FetchError {
    #[error("get logs failed")]
    GetLogs,

    #[error("get tx failed")]
    GetTx,
}

pub struct L1FetcherOptions {
    /// The Ethereum JSON-RPC HTTP URL to use.
    pub http_url: String,
    /// Ethereum block number to start state import from.
    pub start_block: u64,
    /// The number of blocks to filter & process in one step over.
    pub block_step: u64,
    /// The number of blocks to process from Ethereum.
    pub block_count: Option<u64>,
    /// If present, don't poll for new blocks after reaching the end.
    pub disable_polling: bool,
}

#[derive(Default)]
struct L1Metrics {
    // Metrics variables.
    l1_blocks_processed: u64,
    l2_blocks_processed: u64,
    latest_l1_block_nbr: u64,
    latest_l2_block_nbr: u64,

    first_l1_block: u64,
    last_l1_block: u64,
}

impl L1Metrics {
    fn print(&mut self) {
        if self.latest_l1_block_nbr == 0 {
            return;
        }

        let progress = {
            let total = self.last_l1_block - self.first_l1_block;
            let cur = self.latest_l1_block_nbr - self.first_l1_block;
            // If polling past `last_l1_block`, stop at 100%.
            let perc = std::cmp::min((cur * 100) / total, 100);
            format!("{perc:>2}%")
        };

        tracing::info!(
            "PROGRESS: [{}] CUR BLOCK L1: {} L2: {} TOTAL BLOCKS PROCESSED L1: {} L2: {}",
            progress,
            self.latest_l1_block_nbr,
            self.latest_l2_block_nbr,
            self.l1_blocks_processed,
            self.l2_blocks_processed
        );
    }
}

pub struct L1Fetcher {
    provider: Provider<Http>,
    contract: Contract,
    config: L1FetcherOptions,
    snapshot: Option<Arc<Mutex<StateSnapshot>>>,
    metrics: Arc<Mutex<L1Metrics>>,
}

impl L1Fetcher {
    pub fn new(
        config: L1FetcherOptions,
        snapshot: Option<Arc<Mutex<StateSnapshot>>>,
    ) -> Result<Self> {
        let provider = Provider::<Http>::try_from(&config.http_url)
            .expect("could not instantiate HTTP Provider");

        let abi_file = std::fs::File::open("./IZkSync.json")?;
        let contract = Contract::load(abi_file)?;

        let metrics = Arc::new(Mutex::new(L1Metrics::default()));

        Ok(L1Fetcher {
            provider,
            contract,
            config,
            snapshot,
            metrics,
        })
    }

    pub async fn run(&self, sink: mpsc::Sender<CommitBlockInfoV1>) -> Result<()> {
        // Start fetching from the `GENESIS_BLOCK` unless the `start_block` argument is supplied,
        // in which case, start from that instead. If no argument was supplied and a state snapshot
        // exists, start from the block number specified in that snapshot.
        let mut current_l1_block_number = U64::from(self.config.start_block);
        // User might have supplied their own start block, in that case we shouldn't enforce the
        // use of the snapshot value.
        if current_l1_block_number == GENESIS_BLOCK.into() {
            if let Some(snapshot) = &self.snapshot {
                let snapshot = snapshot.lock().await;
                if snapshot.latest_l1_block_number > current_l1_block_number {
                    current_l1_block_number = snapshot.latest_l1_block_number;
                    tracing::info!(
                        "Found snapshot, starting from L1 block {current_l1_block_number}"
                    );
                }
            };
        }

        let end_block = self
            .config
            .block_count
            .map(|count| U64::from(self.config.start_block + count));

        let end_block_number = end_block.unwrap_or(
            self.provider
                .get_block(BlockNumber::Latest)
                .await
                .unwrap()
                .unwrap()
                .number
                .unwrap(),
        );

        // Initialize metrics with last state, if it exists.
        {
            let mut metrics = self.metrics.lock().await;
            metrics.last_l1_block = end_block_number.as_u64();
            metrics.first_l1_block = current_l1_block_number.as_u64();
            metrics.latest_l1_block_nbr = current_l1_block_number.as_u64();
            if let Some(snapshot) = &self.snapshot {
                metrics.latest_l2_block_nbr = snapshot.lock().await.latest_l2_block_number;
            }
        }

        tokio::spawn({
            let metrics = self.metrics.clone();
            async move {
                loop {
                    metrics.lock().await.print();
                    tokio::time::sleep(Duration::from_secs(METRICS_PRINT_INTERVAL_S)).await;
                }
            }
        });

        // Wait for shutdown signal in background.
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Shutdown signal received, finishing up and shutting down...");
            let _ = shutdown_tx.send("");
        });

        let (hash_tx, hash_rx) = mpsc::channel(5);
        let (calldata_tx, calldata_rx) = mpsc::channel(5);

        // If an `end_block` was supplied we shouldn't poll for newer blocks.
        let mut disable_polling = self.config.disable_polling;
        if end_block.is_some() {
            disable_polling = false;
        }

        // Split L1 block processing into three async steps:
        // - BlockCommit event filter (main).
        // - Referred L1 block fetch (tx).
        // - Calldata parsing (parse).
        let tx_handle =
            self.spawn_tx_handler(hash_rx, calldata_tx, current_l1_block_number.as_u64());
        let parse_handle = self.spawn_parsing_handler(calldata_rx, sink)?;
        let main_handle = self.spawn_main_handler(
            hash_tx,
            shutdown_rx,
            current_l1_block_number,
            end_block_number,
            disable_polling,
        )?;

        tx_handle.await?;
        parse_handle.await?;
        main_handle.await?;

        self.metrics.lock().await.print();

        Ok(())
    }

    fn spawn_main_handler(
        &self,
        hash_tx: mpsc::Sender<H256>,
        mut shutdown_rx: oneshot::Receiver<&'static str>,
        mut current_l1_block_number: U64,
        end_block_number: U64,
        disable_polling: bool,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let metrics = self.metrics.clone();
        let event = self.contract.events_by_name("BlockCommit")?[0].clone();
        let provider_clone = self.provider.clone();
        let snapshot_clone = self.snapshot.clone();

        Ok(tokio::spawn({
            async move {
                let mut latest_l2_block_number = U256::zero();

                loop {
                    // Break when reaching the `end_block` or on the receivement of a `ctrl_c` signal.
                    if (disable_polling && current_l1_block_number > end_block_number)
                        || shutdown_rx.try_recv().is_ok()
                    {
                        // Store our current L1 block number so we can resume from where we left
                        // off, we also make sure to update the metrics before leaving the loop.
                        metrics.lock().await.latest_l1_block_nbr = current_l1_block_number.as_u64();
                        if let Some(snapshot) = &snapshot_clone {
                            snapshot.lock().await.latest_l1_block_number = current_l1_block_number;
                        }
                        break;
                    }

                    // Create a filter showing only `BlockCommit`s from the [`ZK_SYNC_ADDR`].
                    // TODO: Filter by executed blocks too.
                    let filter = Filter::new()
                        .address(ZK_SYNC_ADDR.parse::<Address>().unwrap())
                        .topic0(event.signature())
                        .from_block(current_l1_block_number)
                        .to_block(current_l1_block_number + BLOCK_STEP);

                    // Grab all relevant logs.
                    if let Ok(logs) = L1Fetcher::retry_call(
                        || provider_clone.get_logs(&filter),
                        L1FetchError::GetLogs,
                    )
                    .await
                    {
                        for log in logs {
                            // log.topics:
                            // topics[1]: L2 block number.
                            // topics[2]: L2 block hash.
                            // topics[3]: L2 commitment.

                            let new_l2_block_number =
                                U256::from_big_endian(log.topics[1].as_fixed_bytes());
                            if new_l2_block_number <= latest_l2_block_number {
                                continue;
                            }

                            if let Some(tx_hash) = log.transaction_hash {
                                hash_tx.send(tx_hash).await.unwrap();
                            }

                            latest_l2_block_number = new_l2_block_number;
                        }
                    } else {
                        tokio::time::sleep(Duration::from_secs(LONG_POLLING_INTERVAL_S)).await;
                        continue;
                    };

                    metrics.lock().await.latest_l1_block_nbr = current_l1_block_number.as_u64();

                    // Increment current block index.
                    current_l1_block_number += BLOCK_STEP.into();
                }
            }
        }))
    }

    fn spawn_tx_handler(
        &self,
        mut hash_rx: mpsc::Receiver<H256>,
        calldata_tx: mpsc::Sender<Bytes>,
        mut last_block: u64,
    ) -> tokio::task::JoinHandle<()> {
        let metrics = self.metrics.clone();
        let provider = self.provider.clone();

        tokio::spawn({
            async move {
                while let Some(hash) = hash_rx.recv().await {
                    let tx = loop {
                        match L1Fetcher::retry_call(
                            || provider.get_transaction(hash),
                            L1FetchError::GetTx,
                        )
                        .await
                        {
                            Ok(Some(tx)) => {
                                break tx;
                            }
                            _ => {
                                tracing::error!(
                                    "failed to get transaction for hash: {}, retrying in a bit...",
                                    hash
                                );
                                tokio::time::sleep(Duration::from_secs(
                                    FAILED_FETCH_RETRY_INTERVAL_S,
                                ))
                                .await;
                            }
                        };
                    };

                    if let Some(current_block) = tx.block_number {
                        let current_block = current_block.as_u64();
                        if last_block < current_block {
                            let mut metrics = metrics.lock().await;
                            metrics.l1_blocks_processed += current_block - last_block;
                            last_block = current_block;
                        }
                    }

                    calldata_tx.send(tx.input).await.unwrap();
                }
            }
        })
    }

    fn spawn_parsing_handler(
        &self,
        mut calldata_rx: mpsc::Receiver<Bytes>,
        sink: mpsc::Sender<CommitBlockInfoV1>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let metrics = self.metrics.clone();
        let function = self.contract.functions_by_name("commitBlocks")?[0].clone();

        Ok(tokio::spawn({
            async move {
                while let Some(calldata) = calldata_rx.recv().await {
                    let blocks = match parse_calldata(&function, &calldata) {
                        Ok(blks) => blks,
                        Err(e) => {
                            tracing::error!("failed to parse calldata: {e}");
                            continue;
                        }
                    };

                    for blk in blocks {
                        // NOTE: Let's see if we want to increment this in batches, instead of each block individually.
                        let mut metrics = metrics.lock().await;
                        metrics.l2_blocks_processed += 1;
                        metrics.latest_l2_block_nbr = blk.block_number;
                        sink.send(blk).await.unwrap();
                    }
                }
            }
        }))
    }

    async fn retry_call<T, Fut>(callback: impl Fn() -> Fut, err: L1FetchError) -> Result<T>
    where
        Fut: Future<Output = Result<T, ProviderError>>,
    {
        for attempt in 1..=MAX_RETRIES {
            match callback().await {
                Ok(x) => return Ok(x),
                Err(e) => {
                    tracing::error!("attempt {attempt}: failed to fetch from L1: {e}");
                    sleep(Duration::from_millis(50 + random::<u64>() % 500)).await;
                }
            }
        }
        Err(err.into())
    }
}

pub fn parse_calldata(
    commit_blocks_fn: &Function,
    calldata: &[u8],
) -> Result<Vec<CommitBlockInfoV1>> {
    let mut parsed_input = commit_blocks_fn
        .decode_input(&calldata[4..])
        .map_err(|e| ParseError::InvalidCalldata(e.to_string()))?;

    if parsed_input.len() != 2 {
        return Err(ParseError::InvalidCalldata(format!(
            "invalid number of parameters (got {}, expected 2) for commitBlocks function",
            parsed_input.len()
        ))
        .into());
    }

    let new_blocks_data = parsed_input
        .pop()
        .ok_or_else(|| ParseError::InvalidCalldata("new blocks data".to_string()))?;
    let stored_block_info = parsed_input
        .pop()
        .ok_or_else(|| ParseError::InvalidCalldata("stored block info".to_string()))?;

    let abi::Token::Tuple(stored_block_info) = stored_block_info else {
        return Err(ParseError::InvalidCalldata("invalid StoredBlockInfo".to_string()).into());
    };

    let abi::Token::Uint(_previous_l2_block_number) = stored_block_info[0].clone() else {
        return Err(ParseError::InvalidStoredBlockInfo(
            "cannot parse previous L2 block number".to_string(),
        )
        .into());
    };

    let abi::Token::Uint(_previous_enumeration_index) = stored_block_info[2].clone() else {
        return Err(ParseError::InvalidStoredBlockInfo(
            "cannot parse previous enumeration index".to_string(),
        )
        .into());
    };

    //let previous_enumeration_index = previous_enumeration_index.0[0];
    // TODO: What to do here?
    // assert_eq!(previous_enumeration_index, tree.next_enumeration_index());

    parse_commit_block_info(&new_blocks_data)
}

fn parse_commit_block_info(data: &abi::Token) -> Result<Vec<CommitBlockInfoV1>> {
    let mut res = vec![];

    let abi::Token::Array(data) = data else {
        return Err(ParseError::InvalidCommitBlockInfo(
            "cannot convert newBlocksData to array".to_string(),
        )
        .into());
    };

    for d in data {
        match CommitBlockInfoV1::try_from(d) {
            Ok(blk) => res.push(blk),
            Err(e) => tracing::error!("failed to parse commit block info: {e}"),
        }
    }

    Ok(res)
}

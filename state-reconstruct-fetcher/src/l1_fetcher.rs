use std::{fs::File, future::Future, sync::Arc};

use ethers::{
    abi::{Contract, Function},
    prelude::*,
};
use eyre::Result;
use rand::random;
use thiserror::Error;
use tokio::{
    sync::{mpsc, Mutex},
    time::{sleep, Duration, Instant},
};
use tokio_util::sync::CancellationToken;

use crate::{
    constants::ethereum::{BLOCK_STEP, BOOJUM_BLOCK, GENESIS_BLOCK, ZK_SYNC_ADDR},
    database::InnerDB,
    metrics::L1Metrics,
    types::{v1::V1, v2::V2, CommitBlock, ParseError},
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

    #[error("get end block number failed")]
    GetEndBlockNumber,
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

#[derive(Clone)]
struct Contracts {
    v1: Contract,
    v2: Contract,
}

pub struct L1Fetcher {
    provider: Provider<Http>,
    contracts: Contracts,
    config: L1FetcherOptions,
    snapshot: Option<Arc<Mutex<InnerDB>>>,
    metrics: Arc<Mutex<L1Metrics>>,
}

impl L1Fetcher {
    pub fn new(config: L1FetcherOptions, snapshot: Option<Arc<Mutex<InnerDB>>>) -> Result<Self> {
        let provider = Provider::<Http>::try_from(&config.http_url)
            .expect("could not instantiate HTTP Provider");

        let v1 = Contract::load(File::open("./abi/IZkSync.json")?)?;
        let v2 = Contract::load(File::open("./abi/IZkSyncV2.json")?)?;
        let contracts = Contracts { v1, v2 };

        let metrics = Arc::new(Mutex::new(L1Metrics::new(config.start_block)));

        Ok(L1Fetcher {
            provider,
            contracts,
            config,
            snapshot,
            metrics,
        })
    }

    pub async fn run(&self, sink: mpsc::Sender<CommitBlock>) -> Result<()> {
        // Start fetching from the `GENESIS_BLOCK` unless the `start_block` argument is supplied,
        // in which case, start from that instead. If no argument was supplied and a state snapshot
        // exists, start from the block number specified in that snapshot.
        let mut current_l1_block_number = U64::from(self.config.start_block);
        // User might have supplied their own start block, in that case we shouldn't enforce the
        // use of the snapshot value.
        if current_l1_block_number == GENESIS_BLOCK.into() {
            if let Some(snapshot) = &self.snapshot {
                let snapshot = snapshot.lock().await;
                let snapshot_latest_l1_block_number = snapshot.get_latest_l1_block_number()?;
                if snapshot_latest_l1_block_number > current_l1_block_number {
                    current_l1_block_number = snapshot_latest_l1_block_number;
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

        // Initialize metrics with last state, if it exists.
        {
            let mut metrics = self.metrics.lock().await;
            metrics.initial_l1_block = self.config.start_block;
            metrics.first_l1_block_num = current_l1_block_number.as_u64();
            metrics.latest_l1_block_num = current_l1_block_number.as_u64();
            if let Some(snapshot) = &self.snapshot {
                metrics.latest_l2_block_num = snapshot.lock().await.get_latest_l2_block_number()?;
                metrics.first_l2_block_num = metrics.latest_l2_block_num;
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
        let token = CancellationToken::new();
        let cloned_token = token.clone();
        tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    tracing::info!("Shutdown signal received, finishing up and shutting down...");
                }
                Err(err) => {
                    tracing::error!("Shutdown signal failed: {err}");
                }
            };

            cloned_token.cancel();
        });

        let (hash_tx, hash_rx) = mpsc::channel(5);
        let (calldata_tx, calldata_rx) = mpsc::channel(5);

        // If an `end_block` was supplied we shouldn't poll for newer blocks.
        let mut disable_polling = self.config.disable_polling;
        if end_block.is_some() {
            disable_polling = true;
        }

        // Split L1 block processing into three async steps:
        // - BlockCommit event filter (main).
        // - Referred L1 block fetch (tx).
        // - Calldata parsing (parse).
        let tx_handle = self.spawn_tx_handler(
            hash_rx,
            calldata_tx,
            token.clone(),
            current_l1_block_number.as_u64(),
        );
        let parse_handle = self.spawn_parsing_handler(calldata_rx, sink, token.clone())?;
        let main_handle = self.spawn_main_handler(
            hash_tx,
            token,
            current_l1_block_number,
            end_block,
            disable_polling,
        )?;

        tx_handle.await?;
        let last_processed_l1_block_num = parse_handle.await?;
        let mut last_fetched_l1_block_num = main_handle.await?;

        // Store our current L1 block number so we can resume from where we left
        // off, we also make sure to update the metrics before printing them.
        if let Some(block_num) = last_processed_l1_block_num {
            if let Some(snapshot) = &self.snapshot {
                snapshot
                    .lock()
                    .await
                    .set_latest_l1_block_number(block_num)?;
            }

            // Fetching is naturally ahead of parsing, but the data
            // that wasn't parsed on program interruption/error is
            // now lost and will have to be fetched again...
            if last_fetched_l1_block_num > block_num {
                tracing::debug!(
                    "L1 Blocks fetched but not parsed: {}",
                    last_fetched_l1_block_num - block_num
                );
                last_fetched_l1_block_num = block_num;
            }

            let mut metrics = self.metrics.lock().await;
            metrics.latest_l1_block_num = last_fetched_l1_block_num;
            metrics.print();
        } else {
            tracing::warn!("No new blocks were processed");
        }

        Ok(())
    }

    fn spawn_main_handler(
        &self,
        hash_tx: mpsc::Sender<H256>,
        cancellation_token: CancellationToken,
        mut current_l1_block_number: U64,
        mut end_block: Option<U64>,
        disable_polling: bool,
    ) -> Result<tokio::task::JoinHandle<u64>> {
        let metrics = self.metrics.clone();
        let event = self.contracts.v1.events_by_name("BlockCommit")?[0].clone();
        let provider_clone = self.provider.clone();

        Ok(tokio::spawn({
            async move {
                let mut latest_l2_block_number = U256::zero();
                let mut previous_hash = None;
                loop {
                    // Break on the receivement of a `ctrl_c` signal.
                    if cancellation_token.is_cancelled() {
                        tracing::debug!("Shutting down main handler...");
                        break;
                    }

                    let Some(end_block_number) = end_block else {
                        if let Ok(new_end) = L1Fetcher::retry_call(
                            || provider_clone.get_block(BlockNumber::Latest),
                            L1FetchError::GetEndBlockNumber,
                        )
                        .await
                        {
                            if let Some(found_block) = new_end {
                                if let Some(end_block_number) = found_block.number {
                                    end_block = Some(end_block_number);
                                    metrics.lock().await.last_l1_block = end_block_number.as_u64();
                                }
                            }
                        } else {
                            tracing::debug!("Cannot get latest block number...");
                            tokio::time::sleep(Duration::from_secs(LONG_POLLING_INTERVAL_S)).await;
                        }

                        continue;
                    };

                    if current_l1_block_number > end_block_number {
                        // This function must not be called w/ current_l1_block_number > end_block_number .
                        assert!(!disable_polling);
                        tracing::debug!("Waiting for upstream to move on...");
                        tokio::time::sleep(Duration::from_secs(LONG_POLLING_INTERVAL_S)).await;
                        end_block = None;
                        continue;
                    }

                    // Create a filter showing only `BlockCommit`s from the [`ZK_SYNC_ADDR`].
                    // TODO: Filter by executed blocks too.
                    let filter = Filter::new()
                        .address(ZK_SYNC_ADDR.parse::<Address>().unwrap())
                        .topic0(event.signature())
                        .from_block(current_l1_block_number)
                        .to_block(current_l1_block_number + BLOCK_STEP);

                    // Grab all relevant logs.
                    let before = Instant::now();
                    if let Ok(logs) = L1Fetcher::retry_call(
                        || provider_clone.get_logs(&filter),
                        L1FetchError::GetLogs,
                    )
                    .await
                    {
                        let duration = before.elapsed();
                        metrics.lock().await.log_acquisition.add(duration);

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
                                if let Some(prev_hash) = previous_hash {
                                    if prev_hash == tx_hash {
                                        tracing::debug!(
                                            "Transaction hash {:?} already known - not sending.",
                                            tx_hash
                                        );
                                        continue;
                                    }
                                }

                                if let Err(e) = hash_tx.send(tx_hash).await {
                                    if cancellation_token.is_cancelled() {
                                        tracing::debug!("Shutting down tx sender...");
                                    } else {
                                        tracing::error!("Cannot send tx hash: {e}");
                                        cancellation_token.cancel();
                                    }

                                    return current_l1_block_number.as_u64();
                                }

                                previous_hash = Some(tx_hash);
                            }

                            latest_l2_block_number = new_l2_block_number;
                        }
                    } else {
                        tokio::time::sleep(Duration::from_secs(LONG_POLLING_INTERVAL_S)).await;
                        continue;
                    };

                    metrics.lock().await.latest_l1_block_num = current_l1_block_number.as_u64();

                    let next_l1_block_number = current_l1_block_number + U64::from(BLOCK_STEP);
                    if next_l1_block_number > end_block_number {
                        // Some of the `BLOCK_STEP` blocks asked for in this iteration
                        // probably didn't exist yet, so we set `current_l1_block_number`
                        // appropriately as to not skip them.
                        if current_l1_block_number < end_block_number {
                            current_l1_block_number = end_block_number;
                        } else {
                            // `current_l1_block_number == end_block_number`,
                            // IOW, no more blocks can be retrieved until new ones
                            // have been published on L1.
                            if disable_polling {
                                tracing::debug!("Fetching finished...");
                                return current_l1_block_number.as_u64();
                            }

                            current_l1_block_number = end_block_number + U64::one();
                            // current_l1_block_number >
                            // end_block_number, IOW end block will be
                            // reset in the next iteration & updated
                            // afterwards
                        }
                    } else {
                        // We haven't reached past `end_block` yet, stepping by [`BLOCK_STEP`].
                        current_l1_block_number = next_l1_block_number;
                    }
                }

                current_l1_block_number.as_u64()
            }
        }))
    }

    fn spawn_tx_handler(
        &self,
        mut hash_rx: mpsc::Receiver<H256>,
        l1_tx_tx: mpsc::Sender<Transaction>,
        cancellation_token: CancellationToken,
        mut last_block: u64,
    ) -> tokio::task::JoinHandle<()> {
        let metrics = self.metrics.clone();
        let provider = self.provider.clone();

        tokio::spawn({
            async move {
                while let Some(hash) = hash_rx.recv().await {
                    let tx = loop {
                        let before = Instant::now();
                        match L1Fetcher::retry_call(
                            || provider.get_transaction(hash),
                            L1FetchError::GetTx,
                        )
                        .await
                        {
                            Ok(Some(tx)) => {
                                let duration = before.elapsed();
                                metrics.lock().await.tx_acquisition.add(duration);
                                break tx;
                            }
                            _ => {
                                // Task has been cancelled by user, abort loop.
                                if cancellation_token.is_cancelled() {
                                    tracing::debug!("Shutting down tx handler...");
                                    return;
                                }

                                tracing::error!(
                                    "Failed to get transaction for hash: {}, retrying in a bit...",
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
                            metrics.lock().await.latest_l1_block_num = current_block;
                            last_block = current_block;
                        }
                    }

                    if let Err(e) = l1_tx_tx.send(tx).await {
                        if cancellation_token.is_cancelled() {
                            tracing::debug!("Shutting down tx task...");
                        } else {
                            tracing::error!("Cannot send tx: {e}");
                        }

                        return;
                    }
                }
            }
        })
    }

    fn spawn_parsing_handler(
        &self,
        mut l1_tx_rx: mpsc::Receiver<Transaction>,
        sink: mpsc::Sender<CommitBlock>,
        cancellation_token: CancellationToken,
    ) -> Result<tokio::task::JoinHandle<Option<u64>>> {
        let metrics = self.metrics.clone();
        let contracts = self.contracts.clone();
        Ok(tokio::spawn({
            async move {
                let mut boojum_mode = false;
                let mut function =
                    contracts.v1.functions_by_name("commitBlocks").unwrap()[0].clone();
                let mut last_block_number_processed = None;

                while let Some(tx) = l1_tx_rx.recv().await {
                    if cancellation_token.is_cancelled() {
                        tracing::debug!("Shutting down parsing handler...");
                        return last_block_number_processed;
                    }

                    let before = Instant::now();
                    let block_number = tx.block_number.map(|v| v.as_u64());

                    if !boojum_mode {
                        if let Some(block_number) = block_number {
                            if block_number >= BOOJUM_BLOCK {
                                tracing::debug!(
                                    "Reached `BOOJUM_BLOCK`, changing commit block format"
                                );
                                boojum_mode = true;
                                function = contracts.v2.functions_by_name("commitBatches").unwrap()
                                    [0]
                                .clone();
                            }
                        }
                    }

                    let blocks =
                        match parse_calldata(block_number, boojum_mode, &function, &tx.input).await
                        {
                            Ok(blks) => blks,
                            Err(e) => {
                                tracing::error!("Failed to parse calldata: {e}");
                                cancellation_token.cancel();
                                return last_block_number_processed;
                            }
                        };

                    let mut metrics = metrics.lock().await;
                    for blk in blocks {
                        metrics.latest_l2_block_num = blk.l2_block_number;
                        if let Err(e) = sink.send(blk).await {
                            if cancellation_token.is_cancelled() {
                                tracing::debug!("Shutting down parsing task...");
                            } else {
                                tracing::error!("Cannot send block: {e}");
                                cancellation_token.cancel();
                            }

                            return last_block_number_processed;
                        }
                    }

                    last_block_number_processed = block_number;
                    let duration = before.elapsed();
                    metrics.parsing.add(duration);
                }

                // Return the last processed l1 block number, so we can resume from the same point later on.
                last_block_number_processed
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

pub async fn parse_calldata(
    l1_block_number: Option<u64>,
    boojum_mode: bool,
    commit_blocks_fn: &Function,
    calldata: &[u8],
) -> Result<Vec<CommitBlock>> {
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

    // Parse blocks using [`CommitBlockInfoV1`] or [`CommitBlockInfoV2`]
    let mut block_infos = parse_commit_block_info(&new_blocks_data, boojum_mode).await?;
    // Supplement every `CommitBlock` element with L1 block number information.
    block_infos
        .iter_mut()
        .for_each(|e| e.l1_block_number = l1_block_number);
    Ok(block_infos)
}

async fn parse_commit_block_info(
    data: &abi::Token,
    use_new_format: bool,
) -> Result<Vec<CommitBlock>> {
    let abi::Token::Array(data) = data else {
        return Err(ParseError::InvalidCommitBlockInfo(
            "cannot convert newBlocksData to array".to_string(),
        )
        .into());
    };

    let mut result = vec![];
    for d in data {
        let commit_block = if use_new_format {
            CommitBlock::try_from_token::<V2>(d)?
        } else {
            CommitBlock::try_from_token::<V1>(d)?
        };

        result.push(commit_block);
    }

    Ok(result)
}

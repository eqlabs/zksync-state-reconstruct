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
    sync::{mpsc, Mutex},
    time::{sleep, Duration},
};

use crate::{
    constants::ethereum::{BLOCK_STEP, GENESIS_BLOCK, ZK_SYNC_ADDR},
    snapshot::StateSnapshot,
    types::{CommitBlockInfoV1, ParseError},
};

/// MAX_RETRIES is the maximum number of retries on failed L1 call.
const MAX_RETRIES: u8 = 5;

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub enum L1FetchError {
    #[error("get logs failed")]
    GetLogs,

    #[error("get tx failed")]
    GetTx,
}

pub struct L1Fetcher {
    provider: Provider<Http>,
    contract: Contract,
    snapshot: Option<Arc<Mutex<StateSnapshot>>>,
}

impl L1Fetcher {
    pub fn new(http_url: &str, snapshot: Option<Arc<Mutex<StateSnapshot>>>) -> Result<Self> {
        let provider =
            Provider::<Http>::try_from(http_url).expect("could not instantiate HTTP Provider");

        let abi_file = std::fs::File::open("./IZkSync.json")?;
        let contract = Contract::load(abi_file)?;

        Ok(L1Fetcher {
            provider,
            contract,
            snapshot,
        })
    }

    pub async fn fetch(
        &self,
        sink: mpsc::Sender<CommitBlockInfoV1>,
        start_block: Option<U64>,
        end_block: Option<U64>,
    ) -> Result<()> {
        // Start fetching from the `GENESIS_BLOCK` unless the `start_block` argument is supplied,
        // in which case, start from that instead. If no argument was supplied and a state snapshot
        // exists, start from the block number specified in that snapshot.
        let mut current_l1_block_number = start_block.unwrap_or(GENESIS_BLOCK.into());
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

        let end_block_number = end_block.unwrap_or(
            self.provider
                .get_block(BlockNumber::Latest)
                .await?
                .unwrap()
                .number
                .unwrap(),
        );

        let event = self.contract.events_by_name("BlockCommit")?[0].clone();
        let function = self.contract.functions_by_name("commitBlocks")?[0].clone();

        let (hash_tx, mut hash_rx) = mpsc::channel(5);
        let (calldata_tx, mut calldata_rx) = mpsc::channel(5);

        // Split L1 block processing into three async steps:
        // - BlockCommit event filter.
        // - Referred L1 block fetch.
        // - Calldata parsing.
        tokio::spawn({
            let provider = self.provider.clone();
            async move {
                while let Some(hash) = hash_rx.recv().await {
                    let data = match L1Fetcher::retry_call(
                        || provider.get_transaction(hash),
                        L1FetchError::GetTx,
                    )
                    .await
                    {
                        Ok(Some(tx)) => tx.input,
                        _ => continue,
                    };

                    calldata_tx.send(data).await.unwrap();
                }
            }
        });

        tokio::spawn(async move {
            while let Some(calldata) = calldata_rx.recv().await {
                let blocks = match parse_calldata(&function, &calldata) {
                    Ok(blks) => blks,
                    Err(e) => {
                        tracing::error!("failed to parse calldata: {e}");
                        continue;
                    }
                };

                for blk in blocks {
                    sink.send(blk).await.unwrap();
                }
            }
        });

        let mut latest_l2_block_number = U256::zero();
        while current_l1_block_number <= end_block_number {
            // Create a filter showing only `BlockCommit`s from the [`ZK_SYNC_ADDR`].
            // TODO: Filter by executed blocks too.
            let filter = Filter::new()
                .address(ZK_SYNC_ADDR.parse::<Address>()?)
                .topic0(event.signature())
                .from_block(current_l1_block_number)
                .to_block(current_l1_block_number + BLOCK_STEP);

            // Grab all relevant logs.
            let logs =
                L1Fetcher::retry_call(|| self.provider.get_logs(&filter), L1FetchError::GetLogs)
                    .await?;
            for log in logs {
                // log.topics:
                // topics[1]: L2 block number.
                // topics[2]: L2 block hash.
                // topics[3]: L2 commitment.

                let new_l2_block_number = U256::from_big_endian(log.topics[1].as_fixed_bytes());
                if new_l2_block_number <= latest_l2_block_number {
                    continue;
                }

                if let Some(tx_hash) = log.transaction_hash {
                    hash_tx.send(tx_hash).await?;
                }

                latest_l2_block_number = new_l2_block_number;
            }

            // Store our current L1 block number so we can resume if the process exits.
            if let Some(snapshot) = &self.snapshot {
                snapshot.lock().await.latest_l1_block_number = current_l1_block_number;
            }

            // Increment current block index.
            current_l1_block_number += BLOCK_STEP.into();
        }

        Ok(())
    }

    async fn retry_call<T, Fut>(callback: impl Fn() -> Fut, err: L1FetchError) -> Result<T>
    where
        Fut: Future<Output = Result<T, ProviderError>>,
    {
        for attempt in 1..MAX_RETRIES + 1 {
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

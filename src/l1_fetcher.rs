use ethers::{abi::Contract, prelude::*, providers::Provider};
use eyre::Result;
use state_reconstruct::constants::ethereum::{BLOCK_STEP, GENESIS_BLOCK, ZK_SYNC_ADDR};
use state_reconstruct::parse_calldata;
use state_reconstruct::CommitBlockInfoV1;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct L1Fetcher {
    provider: Provider<Http>,
    contract: Contract,
}

impl L1Fetcher {
    pub fn new(http_url: &str) -> Result<Self> {
        let provider =
            Provider::<Http>::try_from(http_url).expect("could not instantiate HTTP Provider");

        let abi_file = std::fs::File::open("./IZkSync.json")?;
        let contract = Contract::load(abi_file)?;

        Ok(L1Fetcher { provider, contract })
    }

    pub async fn fetch(
        &self,
        sink: mpsc::Sender<Arc<Vec<CommitBlockInfoV1>>>,
        start_block: Option<U64>,
        end_block: Option<U64>,
    ) -> Result<()> {
        let mut current_l1_block_number = start_block.unwrap_or(GENESIS_BLOCK.into());
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
        let provider = self.provider.clone();
        tokio::spawn(async move {
            while let Some(hash) = hash_rx.recv().await {
                let tx = match provider.get_transaction(hash).await {
                    Ok(x) => x,
                    Err(e) => {
                        println!("failed to get transaction for hash {}: {}", hash, e);
                        continue;
                    }
                };

                let tx = match tx {
                    Some(tx) => Arc::new(tx.input.to_owned()),
                    None => continue,
                };

                calldata_tx.send(tx).await.unwrap();
            }
        });

        tokio::spawn(async move {
            while let Some(calldata) = calldata_rx.recv().await {
                let blocks = match parse_calldata(&function, calldata.as_ref()) {
                    Ok(blks) => blks,
                    Err(e) => {
                        println!("failed to parse calldata: {}", e);
                        continue;
                    }
                };

                sink.send(Arc::new(blocks)).await.unwrap();
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
            let logs = self.provider.get_logs(&filter).await?;
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

            // Increment current block index.
            current_l1_block_number += BLOCK_STEP.into();
        }

        Ok(())
    }
}

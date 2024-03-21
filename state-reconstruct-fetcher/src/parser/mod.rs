mod calldata_tokens;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use ethers::{
    abi::{self, Function},
    types::Transaction,
};
use eyre::Result;
use tokio::{
    sync::{mpsc, Mutex},
    time::sleep,
};
use tokio_util::sync::CancellationToken;

use self::calldata_tokens::CalldataToken;
use crate::{
    blob_http_client::BlobHttpClient,
    constants::ethereum::{BLOB_BLOCK, BOOJUM_BLOCK},
    l1_fetcher::{Contracts, LONG_POLLING_INTERVAL_S},
    metrics::L1Metrics,
    types::{v1::V1, v2::V2, CommitBlock, ParseError},
};

// TODO: Should use the real types format instead.
enum BatchFormat {
    PreBoojum,
    PostBoojum,
    Blob,
}

impl BatchFormat {
    fn from_l1_block_number(block_number: u64) -> BatchFormat {
        if block_number >= BLOB_BLOCK {
            BatchFormat::Blob
        } else if block_number >= BOOJUM_BLOCK {
            BatchFormat::PostBoojum
        } else {
            BatchFormat::PreBoojum
        }
    }
}

pub struct Parser {
    metrics: Arc<Mutex<L1Metrics>>,
    contracts: Contracts,
    blob_client: BlobHttpClient,
    current_format: BatchFormat,
}

impl Parser {
    pub fn new(
        metrics: Arc<Mutex<L1Metrics>>,
        contracts: Contracts,
        blob_url: String,
        current_l1_block_number: u64,
    ) -> Result<Self> {
        let blob_client = BlobHttpClient::new(blob_url)?;
        let current_format = BatchFormat::from_l1_block_number(current_l1_block_number);
        Ok(Self {
            metrics,
            contracts,
            blob_client,
            current_format,
        })
    }

    pub fn spawn_parsing_handler(
        mut self,
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
                    let Some(block_number) = tx.block_number else {
                        tracing::error!("transaction has no block number");
                        break;
                    };
                    let current_l1_block_number = block_number.as_u64();
                    self.current_format =
                        BatchFormat::from_l1_block_number(current_l1_block_number);

                    if !boojum_mode && current_l1_block_number >= BOOJUM_BLOCK {
                        tracing::debug!("Reached `BOOJUM_BLOCK`, changing commit block format");
                        boojum_mode = true;
                        function =
                            contracts.v2.functions_by_name("commitBatches").unwrap()[0].clone();
                    }

                    let blocks = loop {
                        match self
                            .parse_calldata(&function, &tx.input, current_l1_block_number)
                            .await
                        {
                            Ok(blks) => break blks,
                            Err(e) => match e {
                                ParseError::BlobStorageError(_) => {
                                    if cancellation_token.is_cancelled() {
                                        tracing::debug!("Shutting down parsing...");
                                        return last_block_number_processed;
                                    }
                                    sleep(Duration::from_secs(LONG_POLLING_INTERVAL_S)).await;
                                }
                                ParseError::BlobFormatError(data, inner) => {
                                    tracing::error!("Cannot parse {}: {}", data, inner);
                                    cancellation_token.cancel();
                                    return last_block_number_processed;
                                }
                                _ => {
                                    tracing::error!("Failed to parse calldata: {e}");
                                    cancellation_token.cancel();
                                    return last_block_number_processed;
                                }
                            },
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

                    last_block_number_processed = Some(current_l1_block_number);
                    let duration = before.elapsed();
                    metrics.parsing.add(duration);
                }

                // Return the last processed l1 block number, so we can resume from the same point later on.
                last_block_number_processed
            }
        }))
    }

    pub async fn parse_calldata(
        &self,
        commit_blocks_fn: &Function,
        calldata: &[u8],
        current_l1_block_number: u64,
    ) -> Result<Vec<CommitBlock>, ParseError> {
        let parsed_input = commit_blocks_fn
            .decode_input(&calldata[4..])
            .map_err(|e| ParseError::InvalidCalldata(e.to_string()))?;
        let calldata = CalldataToken::try_from(parsed_input)?;

        // Parse blocks using [`CommitBlockInfoV1`] or [`CommitBlockInfoV2`]
        let mut block_infos = self
            .parse_commit_block_info(&calldata.new_blocks_data.data)
            .await?;
        // Supplement every `CommitBlock` element with L1 block number information.
        block_infos
            .iter_mut()
            .for_each(|e| e.l1_block_number = Some(current_l1_block_number));
        Ok(block_infos)
    }

    async fn parse_commit_block_info(
        &self,
        data: &Vec<abi::Token>,
    ) -> Result<Vec<CommitBlock>, ParseError> {
        let mut result = vec![];
        for d in data {
            let commit_block = {
                match self.current_format {
                    BatchFormat::PreBoojum => CommitBlock::try_from_token::<V1>(d)?,
                    BatchFormat::PostBoojum => CommitBlock::try_from_token::<V2>(d)?,
                    BatchFormat::Blob => {
                        CommitBlock::try_from_token_resolve(d, &self.blob_client).await?
                    }
                }
            };

            result.push(commit_block);
        }

        Ok(result)
    }
}

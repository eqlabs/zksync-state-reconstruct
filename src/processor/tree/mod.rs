pub mod query_tree;
mod tree_wrapper;

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use ethers::types::H256;
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::storage::INNER_DB_NAME,
    database::InnerDB,
    metrics::{PerfMetric, METRICS_TRACING_TARGET},
    types::CommitBlock,
};
use tokio::{
    sync::{mpsc, Mutex},
    time::Instant,
};

use self::tree_wrapper::TreeWrapper;
use super::Processor;

pub type RootHash = H256;

pub struct TreeProcessor {
    /// The internal merkle tree.
    tree: TreeWrapper,
    /// The stored state snapshot.
    snapshot: Arc<Mutex<InnerDB>>,
}

impl TreeProcessor {
    pub async fn new(db_path: PathBuf) -> Result<Self> {
        // If database directory already exists, we try to restore the
        // latest state from a database inside of it.  The state
        // contains the last processed block and a mapping of index to
        // key values.
        let inner_db_path = db_path.join(INNER_DB_NAME);
        let init = !db_path.exists();
        if init {
            tracing::info!("No existing snapshot found, starting from genesis...");
        } else {
            assert!(
                inner_db_path.exists(),
                "missing critical part of the database"
            );
        }

        let new_state = InnerDB::new(inner_db_path.clone())?;
        let snapshot = Arc::new(Mutex::new(new_state));
        let tree = TreeWrapper::new(&db_path, snapshot.clone(), init).await?;

        Ok(Self { tree, snapshot })
    }

    pub fn get_snapshot(&self) -> Arc<Mutex<InnerDB>> {
        self.snapshot.clone()
    }
}

#[async_trait]
impl Processor for TreeProcessor {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlock>) {
        let mut insert_metric = PerfMetric::new("tree_insert");
        let mut snapshot_metric = PerfMetric::new("snapshot");
        while let Some(block) = rx.recv().await {
            // Check if we've already processed this block.
            let latest_l2 = self
                .snapshot
                .lock()
                .await
                .get_latest_l2_block_number()
                .expect("value should default to 0");
            if latest_l2 >= block.l2_block_number {
                tracing::debug!(
                    "Block {} has already been processed, skipping.",
                    block.l2_block_number
                );
                continue;
            }

            let mut before = Instant::now();
            if self.tree.insert_block(&block).await.is_err() {
                return;
            }

            insert_metric.add(before.elapsed());

            // Update snapshot values.
            before = Instant::now();
            self.snapshot
                .lock()
                .await
                .set_latest_l2_block_number(block.l2_block_number)
                .expect("db failed");

            if snapshot_metric.add(before.elapsed()) > 10 {
                let insert_avg = insert_metric.reset();
                let snapshot_avg = snapshot_metric.reset();
                tracing::debug!(
                    target: METRICS_TRACING_TARGET,
                    "PERSISTENCE: avg insert {} snapshot {}",
                    insert_avg,
                    snapshot_avg
                );
            }
        }
    }
}

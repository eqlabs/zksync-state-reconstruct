pub mod query_tree;
pub mod tree_wrapper;

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use ethers::types::H256;
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::storage::INNER_DB_NAME,
    metrics::{PerfMetric, METRICS_TRACING_TARGET},
    types::CommitBlock,
};
use state_reconstruct_storage::reconstruction::ReconstructionDatabase;
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
    inner_db: Arc<Mutex<ReconstructionDatabase>>,
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

        let new_state = ReconstructionDatabase::new(inner_db_path.clone())?;
        let inner_db = Arc::new(Mutex::new(new_state));
        let tree = TreeWrapper::new(&db_path, inner_db.clone(), init).await?;

        Ok(Self { tree, inner_db })
    }

    pub fn get_inner_db(&self) -> Arc<Mutex<ReconstructionDatabase>> {
        self.inner_db.clone()
    }
}

#[async_trait]
impl Processor for TreeProcessor {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlock>) {
        let mut insert_metric = PerfMetric::new("tree_insert");
        let mut snapshot_metric = PerfMetric::new("snapshot");
        while let Some(block) = rx.recv().await {
            // Check if we've already processed this block.
            let latest_l1_batch = self
                .inner_db
                .lock()
                .await
                .get_latest_l1_batch_number()
                .expect("value should default to 0");
            if latest_l1_batch >= block.l1_batch_number {
                tracing::debug!(
                    "Batch {} has already been processed, skipping.",
                    block.l1_batch_number
                );
                continue;
            }

            let mut before = Instant::now();
            if self.tree.insert_block(&block).await.is_err() {
                tracing::warn!("Shutting down tree processor...");
                return;
            }

            insert_metric.add(before.elapsed());

            // Update snapshot values.
            before = Instant::now();
            self.inner_db
                .lock()
                .await
                .set_latest_l1_batch_number(block.l1_batch_number)
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

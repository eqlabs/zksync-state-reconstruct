pub mod query_tree;
mod tree_wrapper;

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use ethers::types::H256;
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::storage::STATE_FILE_NAME, perf_metric::PerfMetric, snapshot::StateSnapshot,
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
    snapshot: Arc<Mutex<StateSnapshot>>,
}

impl TreeProcessor {
    pub async fn new(db_path: PathBuf, snapshot: Arc<Mutex<StateSnapshot>>) -> Result<Self> {
        // If database directory already exists, we try to restore the latest state.
        // The state contains the last processed block and a mapping of index to key
        // values, if a state file does not exist, we simply use the defaults instead.
        let should_restore_state = db_path.exists();
        if should_restore_state {
            tracing::info!("Loading previous state file...");
            let new_state = StateSnapshot::read(&db_path.join(STATE_FILE_NAME))
                .expect("state file is malformed");
            *snapshot.lock().await = new_state;
        } else {
            tracing::info!("No existing database found, starting from genesis...");
        };

        // Extract `index_to_key_map` from state snapshot.
        let index_to_key_map = snapshot.lock().await.index_to_key_map.clone();
        let tree = TreeWrapper::new(&db_path, index_to_key_map)?;

        Ok(Self { tree, snapshot })
    }
}

#[async_trait]
impl Processor for TreeProcessor {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlock>) {
        let mut metric = PerfMetric::default();
        while let Some(block) = rx.recv().await {
            let before = Instant::now();

            let mut snapshot = self.snapshot.lock().await;
            // Check if we've already processed this block.
            if snapshot.latest_l2_block_number >= block.l2_block_number {
                tracing::debug!(
                    "Block {} has already been processed, skipping.",
                    block.l2_block_number
                );
                continue;
            }

            self.tree.insert_block(&block);

            // Update snapshot values.
            snapshot.latest_l2_block_number = block.l2_block_number;
            snapshot.index_to_key_map = self.tree.index_to_key_map.clone();

            let after = Instant::now();
            let duration = after.duration_since(before);
            if metric.add(duration) > 10 {
                let avg = metric.renew();
                tracing::info!("BACKGROUND: avg snapshot {}", avg);
            }
        }
    }
}

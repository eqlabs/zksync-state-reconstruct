pub mod query_tree;
mod tree_wrapper;

use std::{io, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use ethers::types::H256;
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::storage::STATE_FILE_NAME, snapshot::StateSnapshot, types::CommitBlockInfoV1,
};
use tokio::sync::{mpsc, Mutex};

use self::tree_wrapper::TreeWrapper;
use super::Processor;

pub type RootHash = H256;

pub struct TreeProcessor<'a> {
    /// The path to the directory in which database files and state snapshots will be written.
    db_path: PathBuf,
    /// The internal merkle tree.
    tree: TreeWrapper<'a>,
    /// The stored state snapshot.
    snapshot: Arc<Mutex<StateSnapshot>>,
}

impl TreeProcessor<'static> {
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

        Ok(Self {
            db_path,
            tree,
            snapshot,
        })
    }

    pub async fn write_state(&self) -> Result<(), io::Error> {
        let snapshot = self.snapshot.lock().await;
        // Write the current state to a file.
        let state_file_path = self.db_path.join(STATE_FILE_NAME);
        snapshot.write(&state_file_path)
    }
}

#[async_trait]
impl Processor for TreeProcessor<'static> {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlockInfoV1>) {
        loop {
            if let Some(block) = rx.recv().await {
                let mut snapshot = self.snapshot.lock().await;
                // Check if we've already processed this block.
                if snapshot.latest_l2_block_number >= block.block_number {
                    tracing::debug!(
                        "Block {} has already been processed, skipping.",
                        block.block_number
                    );
                    continue;
                }

                self.tree.insert_block(&block);

                // Update snapshot values.
                snapshot.latest_l2_block_number = block.block_number;
                snapshot.index_to_key_map = self.tree.index_to_key_map.clone();
            } else {
                self.write_state().await.unwrap();
                break;
            }
        }
    }
}

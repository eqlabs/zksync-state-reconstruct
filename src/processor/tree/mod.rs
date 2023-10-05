mod snapshot;
mod tree_wrapper;

use std::path::Path;

use async_trait::async_trait;
use eyre::Result;
use tokio::sync::mpsc;

use self::{snapshot::StateSnapshot, tree_wrapper::TreeWrapper};
use super::Processor;
use crate::{constants::storage::STATE_FILE_PATH, types::CommitBlockInfoV1};

pub struct TreeProcessor<'a> {
    tree: TreeWrapper<'a>,
    snapshot: StateSnapshot,
}

impl TreeProcessor<'static> {
    pub fn new(db_dir: &Path) -> Result<Self> {
        // TODO: Implement graceful shutdown.
        // If database directory already exists, we try to restore the latest state.
        // The state contains the last processed block and a mapping of index to key
        // values, if a state file does not exist, we simply use the defaults instead.
        let should_restore_state = db_dir.exists();
        let snapshot = if should_restore_state {
            println!("Loading previous state file...");
            StateSnapshot::read(STATE_FILE_PATH).expect("state file is malformed")
        } else {
            println!("No existing database found, starting from genesis...");
            StateSnapshot::default()
        };

        // Extract `index_to_key_map` from state snapshot.
        let StateSnapshot {
            ref index_to_key_map,
            .. // Ignore the rest of the fields.
        } = snapshot;

        let tree = TreeWrapper::new(db_dir, index_to_key_map.clone())?;

        Ok(Self { tree, snapshot })
    }
}

#[async_trait]
impl Processor for TreeProcessor<'static> {
    async fn run(mut self, mut rx: mpsc::Receiver<Vec<CommitBlockInfoV1>>) {
        while let Some(blocks) = rx.recv().await {
            for block in blocks {
                // Check if we've already processed this block.
                if self.snapshot.latest_l2_block_number >= block.block_number {
                    println!(
                        "Block {} has already been processed, skipping.",
                        block.block_number
                    );
                    continue;
                }

                self.tree.insert_block(&block);

                // Update snapshot values.
                self.snapshot.latest_l2_block_number = block.block_number;
                self.snapshot.index_to_key_map = self.tree.index_to_key_map.clone();
            }

            // Write the current state to a file.
            self.snapshot.write(STATE_FILE_PATH).unwrap();
        }
    }
}

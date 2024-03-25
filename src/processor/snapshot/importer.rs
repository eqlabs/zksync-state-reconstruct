#![allow(warnings)]
use std::path::PathBuf;

use state_reconstruct_fetcher::types::CommitBlock;
use tokio::sync::mpsc;

pub struct SnapshotImporter {
    // The path of the directory where snapshot chunks are stored.
    directory: PathBuf,
}

impl SnapshotImporter {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub async fn run(mut self, mut tx: mpsc::Sender<CommitBlock>) {
        todo!()
    }
}

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::types::CommitBlockInfoV1;

pub mod tree;

#[async_trait]
pub trait Processor {
    async fn run(self, rx: mpsc::Receiver<Vec<CommitBlockInfoV1>>);
}

use async_trait::async_trait;
use state_reconstruct_fetcher::types::CommitBlockInfoV1;
use tokio::sync::mpsc;

pub mod json;
pub mod tree;

#[async_trait]
pub trait Processor {
    async fn run(self, rx: mpsc::Receiver<CommitBlockInfoV1>);
}

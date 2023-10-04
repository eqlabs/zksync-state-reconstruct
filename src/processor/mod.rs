use tokio::{sync::mpsc, task::JoinHandle};

use crate::types::CommitBlockInfoV1;

pub mod tree;

pub trait Processor {
    fn run(self, rx: mpsc::Receiver<Vec<CommitBlockInfoV1>>) -> JoinHandle<()>;
}

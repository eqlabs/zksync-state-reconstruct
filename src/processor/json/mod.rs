use std::{fs::File, path::Path};

use async_trait::async_trait;
use eyre::Result;
use serde::ser::{SerializeSeq, Serializer};
use serde_json;
use state_reconstruct_fetcher::types::CommitBlockInfoV1;
use tokio::sync::mpsc;

use super::Processor;

pub struct JsonSerializationProcessor {
    serializer: serde_json::Serializer<File>,
}

impl JsonSerializationProcessor {
    pub fn new(out_file: &Path) -> Result<Self> {
        let file = File::create(out_file)?;
        let serializer = serde_json::Serializer::new(file);
        Ok(Self { serializer })
    }
}

#[async_trait]
impl Processor for JsonSerializationProcessor {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlockInfoV1>) {
        let mut seq = self
            .serializer
            .serialize_seq(None)
            .expect("serializer construction failed");
        while let Some(block) = rx.recv().await {
            seq.serialize_element(&block).expect("block serialization");
        }
        seq.end().expect("JSON array closing");
    }
}

#![allow(warnings)]
use std::{fs, io::Read, path::PathBuf};

use eyre::Result;
use flate2::read::GzDecoder;
use prost::Message;
use state_reconstruct_fetcher::types::CommitBlock;
use tokio::sync::mpsc;

use super::{
    exporter::protobuf::SnapshotFactoryDependencies, types::SnapshotHeader,
    SNAPSHOT_FACTORY_DEPS_FILE_NAME, SNAPSHOT_HEADER_FILE_NAME,
};

pub struct SnapshotImporter {
    // The path of the directory where snapshot chunks are stored.
    directory: PathBuf,
}

impl SnapshotImporter {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub async fn run(mut self, mut tx: mpsc::Sender<CommitBlock>) -> Result<()> {
        let header = self.read_header()?;
        let factory_deps = self.read_factory_deps()?;

        Ok(())
    }

    fn read_header(&self) -> Result<SnapshotHeader> {
        let header_path = self.directory.join(SNAPSHOT_HEADER_FILE_NAME);
        let header_string = fs::read_to_string(header_path)?;
        let header: SnapshotHeader = serde_json::from_str(&header_string)?;

        Ok(header)
    }

    fn read_factory_deps(&self) -> Result<SnapshotFactoryDependencies> {
        let factory_deps_path = self.directory.join(SNAPSHOT_FACTORY_DEPS_FILE_NAME);
        let bytes = fs::read(factory_deps_path)?;
        let mut decoder = GzDecoder::new(&bytes[..]);

        let mut decompressed_bytes = Vec::new();
        decoder.read_to_end(&mut decompressed_bytes)?;

        let factory_deps = SnapshotFactoryDependencies::decode(&decompressed_bytes[..])?;

        Ok(factory_deps)
    }
}

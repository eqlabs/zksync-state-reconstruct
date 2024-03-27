#![allow(warnings)]
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use ethers::types::U256;
use eyre::Result;
use flate2::read::GzDecoder;
use indexmap::IndexMap;
use prost::Message;
use state_reconstruct_fetcher::{
    constants::storage::INNER_DB_NAME,
    database::InnerDB,
    types::{CommitBlock, PackingType},
};
use tokio::sync::{mpsc, Mutex};

use super::{
    database::SnapshotDB,
    exporter::protobuf::{
        SnapshotFactoryDependencies, SnapshotStorageLog, SnapshotStorageLogsChunk,
    },
    types::SnapshotHeader,
    SNAPSHOT_FACTORY_DEPS_FILE_NAME, SNAPSHOT_HEADER_FILE_NAME,
};
use crate::processor::tree::tree_wrapper::TreeWrapper;

pub struct SnapshotImporter {
    // The path of the directory where snapshot chunks are stored.
    directory: PathBuf,
    // The tree to import state to.
    tree: TreeWrapper,
}

impl SnapshotImporter {
    pub async fn new(directory: PathBuf, db_path: &Path) -> Result<Self> {
        let inner_db_path = db_path.join(INNER_DB_NAME);
        // NOTE: Remove dep on snapshot?
        let new_state = InnerDB::new(inner_db_path.clone())?;
        let snapshot = Arc::new(Mutex::new(new_state));
        let tree = TreeWrapper::new(db_path, snapshot.clone(), true).await?;

        Ok(Self { directory, tree })
    }

    pub async fn run(mut self) -> Result<()> {
        let header = self.read_header()?;
        let factory_deps = self.read_factory_deps()?;
        let storage_logs_chunk = self.read_storage_logs_chunks()?;

        self.tree
            .restore_from_snapshot(storage_logs_chunk, header.l1_batch_number)
            .await
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

    fn read_storage_logs_chunks(&self) -> Result<SnapshotStorageLogsChunk> {
        let factory_deps_path = self.directory.join("1.gz");
        let bytes = fs::read(factory_deps_path)?;
        let mut decoder = GzDecoder::new(&bytes[..]);

        let mut decompressed_bytes = Vec::new();
        decoder.read_to_end(&mut decompressed_bytes)?;

        let storage_logs_chunk = SnapshotStorageLogsChunk::decode(&decompressed_bytes[..])?;
        Ok(storage_logs_chunk)
    }
}

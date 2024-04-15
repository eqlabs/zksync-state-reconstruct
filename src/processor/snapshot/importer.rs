use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use eyre::Result;
use state_reconstruct_fetcher::constants::storage::INNER_DB_NAME;
use state_reconstruct_storage::{
    types::{Proto, SnapshotFactoryDependencies, SnapshotHeader, SnapshotStorageLogsChunk},
    InnerDB,
};
use tokio::sync::Mutex;

use super::{SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX, SNAPSHOT_HEADER_FILE_NAME};
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
        let new_state = InnerDB::new(inner_db_path.clone())?;
        let snapshot = Arc::new(Mutex::new(new_state));
        let tree = TreeWrapper::new(db_path, snapshot.clone(), true).await?;

        Ok(Self { directory, tree })
    }

    pub async fn run(mut self) -> Result<()> {
        let header = self.read_header()?;
        let _factory_deps = self.read_factory_deps(&header)?;
        let storage_logs_chunk = self.read_storage_logs_chunks(&header)?;

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

    fn read_factory_deps(&self, header: &SnapshotHeader) -> Result<SnapshotFactoryDependencies> {
        let factory_deps_path = self.directory.join(format!(
            "snapshot_l1_batch_{}_{}",
            header.l1_batch_number, SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX
        ));
        let bytes = fs::read(factory_deps_path)?;
        SnapshotFactoryDependencies::decode(&bytes)
    }

    fn read_storage_logs_chunks(
        &self,
        header: &SnapshotHeader,
    ) -> Result<Vec<SnapshotStorageLogsChunk>> {
        // NOTE: I think these are sorted by default, but if not, we need to sort them
        // before extracting the filepaths.
        let filepaths = header
            .storage_logs_chunks
            .iter()
            .map(|meta| PathBuf::from(&meta.filepath));

        let mut chunks = Vec::with_capacity(filepaths.len());
        for path in filepaths {
            let factory_deps_path = self
                .directory
                .join(path.file_name().expect("path has no file name"));
            let bytes = fs::read(factory_deps_path)?;
            let storage_logs_chunk = SnapshotStorageLogsChunk::decode(&bytes)?;
            chunks.push(storage_logs_chunk);
        }
        Ok(chunks)
    }
}

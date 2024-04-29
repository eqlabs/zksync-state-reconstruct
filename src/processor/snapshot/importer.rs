use std::{
    fs::{self, DirEntry},
    path::{Path, PathBuf},
};

use ethers::types::U64;
use eyre::Result;
use regex::{Captures, Regex};
use state_reconstruct_storage::types::{
    Proto, SnapshotFactoryDependencies, SnapshotHeader, SnapshotStorageLogsChunk,
    SnapshotStorageLogsChunkMetadata,
};
use tokio::sync::mpsc::{self, Sender};

use super::{SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX, SNAPSHOT_HEADER_FILE_NAME};
use crate::processor::tree::tree_wrapper::TreeWrapper;

const SNAPSHOT_CHUNK_REGEX: &str = r"snapshot_l1_batch_(\d*)_storage_logs_part_\d*.proto.gzip";
const FACTORY_DEPS_REGEX: &str = r"snapshot_l1_batch_(\d*)_factory_deps.proto.gzip";

pub struct SnapshotImporter {
    // The path of the directory where snapshot chunks are stored.
    directory: PathBuf,
}

impl SnapshotImporter {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub async fn run(self, db_path: &Path) -> Result<()> {
        let (tx, rx) = mpsc::channel(1);

        let header = self.read_header()?;
        let _factory_deps = self.read_factory_deps(&header)?;

        // Read storage logs async sending each read one into the tree to process.
        tokio::spawn({
            let header = header.clone();
            async move {
                self.read_storage_logs_chunks_async(&header, tx)
                    .await
                    .expect("failed to read storage_logs_chunks");
            }
        });

        let mut tree = TreeWrapper::new_snapshot_wrapper(db_path)
            .await
            .expect("can't create tree");
        tree.restore_from_snapshot(rx, header.l1_batch_number)
            .await?;

        Ok(())
    }

    fn read_header(&self) -> Result<SnapshotHeader> {
        let header_path = self.directory.join(SNAPSHOT_HEADER_FILE_NAME);

        let header = if let Ok(string) = fs::read_to_string(header_path) {
            serde_json::from_str(&string)?
        } else {
            self.infer_header_from_file_names()?
        };

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

    async fn read_storage_logs_chunks_async(
        &self,
        header: &SnapshotHeader,
        tx: Sender<SnapshotStorageLogsChunk>,
    ) -> Result<()> {
        // NOTE: I think these are sorted by default, but if not, we need to sort them
        // before extracting the filepaths.
        let filepaths = header
            .storage_logs_chunks
            .iter()
            .map(|meta| PathBuf::from(&meta.filepath));

        let total_chunks = filepaths.len();
        for (i, path) in filepaths.into_iter().enumerate() {
            let factory_deps_path = self
                .directory
                .join(path.file_name().expect("path has no file name"));
            let bytes = fs::read(factory_deps_path)?;
            let storage_logs_chunk = SnapshotStorageLogsChunk::decode(&bytes)?;
            tracing::info!("Read chunk {}/{}, processing...", i + 1, total_chunks);
            tx.send(storage_logs_chunk).await?;
        }

        Ok(())
    }

    fn infer_header_from_file_names(&self) -> Result<SnapshotHeader> {
        let snapshot_chunk_re = Regex::new(SNAPSHOT_CHUNK_REGEX)?;
        let factory_deps_re = Regex::new(FACTORY_DEPS_REGEX)?;

        let mut l1_batch_number = None;
        let mut storage_logs_chunks = Vec::new();
        let mut factory_deps_filepath = String::new();

        // Closure to make sure that every file name contains the same l1 batch number, assinging
        // one if it is currently set to [`None`].
        let mut process_l1_number = |caps: Captures| {
            let number: u64 = caps[1].parse().expect("capture was not a number");
            match l1_batch_number {
                Some(num) => assert_eq!(num, U64::from(number)),
                None => l1_batch_number = Some(U64::from(number)),
            }
        };

        // Read files and sort them by path name.
        let mut files: Vec<_> = self
            .directory
            .read_dir()?
            .map(|f| f.expect("read file error"))
            .collect();
        files.sort_by_key(DirEntry::path);

        let mut chunk_id = 0;
        for file in files {
            let file_name = file
                .file_name()
                .to_str()
                .expect("invalid filename")
                .to_string();

            if let Some(caps) = snapshot_chunk_re.captures(&file_name) {
                process_l1_number(caps);

                // Add the storage log parts filename to header.
                let chunk_metadata = SnapshotStorageLogsChunkMetadata {
                    chunk_id,
                    filepath: file_name,
                };
                storage_logs_chunks.push(chunk_metadata);
                chunk_id += 1;
            } else if let Some(caps) = factory_deps_re.captures(&file_name) {
                process_l1_number(caps);
                factory_deps_filepath = file_name;
            }
        }

        Ok(SnapshotHeader {
            l1_batch_number: l1_batch_number.expect("no l1 batch number found"),
            storage_logs_chunks,
            factory_deps_filepath,
            ..Default::default()
        })
    }
}

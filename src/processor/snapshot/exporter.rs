use std::path::{Path, PathBuf};

use ethers::types::U64;
use eyre::Result;

use super::{
    database::{self, SnapshotDB},
    types::{SnapshotFactoryDependency, SnapshotHeader},
    DEFAULT_DB_PATH, SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX, SNAPSHOT_HEADER_FILE_NAME,
};
use crate::processor::snapshot::types::{
    Proto, SnapshotFactoryDependencies, SnapshotStorageLogsChunk, SnapshotStorageLogsChunkMetadata,
};

pub struct SnapshotExporter {
    basedir: PathBuf,
    database: SnapshotDB,
}

impl SnapshotExporter {
    pub fn new(basedir: &Path, db_path: Option<String>) -> Result<Self> {
        let db_path = match db_path {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_DB_PATH),
        };

        let database = SnapshotDB::new_read_only(db_path)?;
        Ok(Self {
            basedir: basedir.to_path_buf(),
            database,
        })
    }

    pub fn export_snapshot(&self, chunk_size: u64) -> Result<()> {
        let l1_batch_number = U64::from(
            self.database
                .get_last_l1_batch_number()?
                .expect("snapshot db contains no L1 batch number"),
        );

        let mut header = SnapshotHeader {
            l1_batch_number,
            ..Default::default()
        };

        self.export_storage_logs(chunk_size, &mut header)?;
        self.export_factory_deps(&mut header)?;

        let path = self.basedir.join(SNAPSHOT_HEADER_FILE_NAME);
        let outfile = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        serde_json::to_writer(outfile, &header)?;

        Ok(())
    }

    fn export_factory_deps(&self, header: &mut SnapshotHeader) -> Result<()> {
        tracing::info!("Exporting factory dependencies...");

        let storage_logs = self.database.cf_handle(database::FACTORY_DEPS).unwrap();
        let mut iterator = self
            .database
            .iterator_cf(storage_logs, rocksdb::IteratorMode::Start);

        let mut factory_deps = SnapshotFactoryDependencies::default();
        while let Some(Ok((_, bs))) = iterator.next() {
            let factory_dep: SnapshotFactoryDependency = bincode::deserialize(&bs)?;
            factory_deps.factory_deps.push(factory_dep);
        }

        let path = self.basedir.join(format!(
            "snapshot_l1_batch_{}_{}",
            header.l1_batch_number, SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX
        ));
        header.factory_deps_filepath = path
            .clone()
            .into_os_string()
            .into_string()
            .expect("path to string");

        factory_deps.encode(&path)?;
        tracing::info!("All factory dependencies were successfully serialized!");
        Ok(())
    }

    fn export_storage_logs(&self, chunk_size: u64, header: &mut SnapshotHeader) -> Result<()> {
        tracing::info!("Exporting storage logs...");

        let num_logs = self.database.get_last_repeated_key_index()?;
        tracing::info!("Found {num_logs} logs.");

        let index_to_key_map = self.database.cf_handle(database::INDEX_TO_KEY_MAP).unwrap();
        let mut iterator = self
            .database
            .iterator_cf(index_to_key_map, rocksdb::IteratorMode::Start);

        let total_num_chunks = (num_logs / chunk_size) + 1;
        for chunk_id in 0..total_num_chunks {
            tracing::info!("Serializing chunk {}/{}...", chunk_id + 1, total_num_chunks);

            let mut chunk = SnapshotStorageLogsChunk::default();
            for _ in 0..chunk_size {
                if let Some(Ok((_, key))) = iterator.next() {
                    if let Ok(Some(entry)) = self.database.get_storage_log(key.as_ref()) {
                        chunk.storage_logs.push(entry);
                    }
                } else {
                    break;
                }
            }

            let path = self.basedir.join(format!(
                "snapshot_l1_batch_{}_storage_logs_part_{:0>4}.proto.gzip",
                header.l1_batch_number, chunk_id
            ));
            header
                .storage_logs_chunks
                .push(SnapshotStorageLogsChunkMetadata {
                    chunk_id,
                    filepath: path
                        .clone()
                        .into_os_string()
                        .into_string()
                        .expect("path to string"),
                });

            chunk.encode(&path)?;
            tracing::info!("Chunk {} was successfully serialized!", chunk_id + 1);
        }

        tracing::info!("All storage logs were successfully serialized!");
        Ok(())
    }
}
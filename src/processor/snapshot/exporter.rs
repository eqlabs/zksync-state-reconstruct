use std::path::{Path, PathBuf};

use ethers::types::{U256, U64};
use eyre::{OptionExt, Result};
use state_reconstruct_storage::{
    snapshot::SnapshotDatabase,
    snapshot_columns,
    types::{
        Proto, SnapshotFactoryDependencies, SnapshotFactoryDependency, SnapshotHeader,
        SnapshotStorageLogsChunk, SnapshotStorageLogsChunkMetadata,
    },
    INDEX_TO_KEY_MAP,
};

use crate::processor::snapshot::{
    DEFAULT_DB_PATH, SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX, SNAPSHOT_HEADER_FILE_NAME,
};

pub struct SnapshotExporter {
    basedir: PathBuf,
    database: SnapshotDatabase,
}

impl SnapshotExporter {
    pub fn new(basedir: &Path, db_path: Option<String>) -> Result<Self> {
        let db_path = match db_path {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_DB_PATH),
        };

        let database = SnapshotDatabase::new_read_only(db_path)?;
        Ok(Self {
            basedir: basedir.to_path_buf(),
            database,
        })
    }

    pub fn export_snapshot(&self, num_chunks: usize) -> Result<()> {
        let l1_batch_number = self
            .database
            .get_latest_l1_batch_number()?
            .ok_or_eyre("no latest l1 batch number in snapshot db")?;
        let l2_block_number = self.database.get_latest_l2_block_number()?.unwrap_or_else(|| {
            tracing::warn!("WARNING: the database contains no l2 block number entry and will not be compatible with the ZKSync External Node! To export a compatible snapshot, please let the prepare-snapshot command run until an l2 block number can be found.");
            U64::from(0)
        });
        let mut header = SnapshotHeader {
            l1_batch_number: l1_batch_number.as_u64(),
            miniblock_number: l2_block_number.as_u64(),
            ..Default::default()
        };

        self.export_storage_logs(num_chunks, &mut header)?;
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

        let storage_logs = self
            .database
            .cf_handle(snapshot_columns::FACTORY_DEPS)
            .unwrap();
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

    fn export_storage_logs(&self, num_chunks: usize, header: &mut SnapshotHeader) -> Result<()> {
        tracing::info!("Exporting storage logs...");

        let num_logs = self.database.get_last_repeated_key_index()?;
        tracing::info!("Found {num_logs} logs.");

        let index_to_key_map = self.database.cf_handle(INDEX_TO_KEY_MAP).unwrap();
        let mut iterator = self
            .database
            .iterator_cf(index_to_key_map, rocksdb::IteratorMode::Start);

        let chunk_size = num_logs / num_chunks as u64;
        for chunk_id in 0..num_chunks {
            tracing::info!("Serializing chunk {}/{}...", chunk_id + 1, num_chunks);

            let mut chunk = SnapshotStorageLogsChunk::default();
            for _ in 0..chunk_size {
                if let Some(Ok((_, key))) = iterator.next() {
                    let key = U256::from_big_endian(&key);
                    if let Ok(Some(entry)) = self.database.get_storage_log(&key) {
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
                    chunk_id: chunk_id as u64,
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

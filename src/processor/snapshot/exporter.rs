use std::{
    io::Write,
    path::{Path, PathBuf},
};

use bytes::BytesMut;
use eyre::Result;
use flate2::{write::GzEncoder, Compression};
use prost::Message;

use super::{
    database::{self, SnapshotDB},
    types::{self, SnapshotFactoryDependency, SnapshotHeader},
    DEFAULT_DB_PATH, SNAPSHOT_FACTORY_DEPS_FILE_NAME, SNAPSHOT_HEADER_FILE_NAME,
};

pub mod protobuf {
    include!(concat!(env!("OUT_DIR"), "/protobuf.rs"));
}

pub struct SnapshotExporter {
    basedir: PathBuf,
    database: SnapshotDB,
}

impl SnapshotExporter {
    pub fn new(basedir: &Path, db_path: Option<String>) -> Self {
        let db_path = match db_path {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_DB_PATH),
        };

        let database = SnapshotDB::new_read_only(db_path).unwrap();
        Self {
            basedir: basedir.to_path_buf(),
            database,
        }
    }

    pub fn export_snapshot(&self, chunk_size: u64) -> Result<()> {
        let mut header = SnapshotHeader::default();
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
        let mut buf = BytesMut::new();

        let storage_logs = self.database.cf_handle(database::FACTORY_DEPS).unwrap();
        let mut iterator = self
            .database
            .iterator_cf(storage_logs, rocksdb::IteratorMode::Start);

        let mut factory_deps = protobuf::SnapshotFactoryDependencies::default();
        while let Some(Ok((_, bs))) = iterator.next() {
            let factory_dep: SnapshotFactoryDependency = bincode::deserialize(&bs)?;
            factory_deps
                .factory_deps
                .push(protobuf::SnapshotFactoryDependency {
                    bytecode: Some(factory_dep.bytecode),
                });
        }

        let fd_len = factory_deps.encoded_len();
        if buf.capacity() < fd_len {
            buf.reserve(fd_len - buf.capacity());
        }

        let path = self.basedir.join(SNAPSHOT_FACTORY_DEPS_FILE_NAME);
        header.factory_deps_filepath = path
            .clone()
            .into_os_string()
            .into_string()
            .expect("path to string");

        // Serialize chunk.
        factory_deps.encode(&mut buf)?;

        let outfile = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        // Wrap in gzip compression before writing.
        let mut encoder = GzEncoder::new(outfile, Compression::default());
        encoder.write_all(&buf)?;
        encoder.finish()?;

        Ok(())
    }

    fn export_storage_logs(&self, chunk_size: u64, header: &mut SnapshotHeader) -> Result<()> {
        let mut buf = BytesMut::new();
        let mut chunk_index = 0;

        let index_to_key_map = self.database.cf_handle(database::INDEX_TO_KEY_MAP).unwrap();
        let mut iterator = self
            .database
            .iterator_cf(index_to_key_map, rocksdb::IteratorMode::Start);

        let mut has_more = true;

        while has_more {
            let mut chunk = protobuf::SnapshotStorageLogsChunk {
                storage_logs: vec![],
            };

            for _ in 0..chunk_size {
                if let Some(Ok((_, key))) = iterator.next() {
                    if let Ok(Some(entry)) = self.database.get_storage_log(key.as_ref()) {
                        let pb = protobuf::SnapshotStorageLog {
                            account_address: None,
                            storage_key: Some(key.to_vec()),
                            storage_value: Some(entry.value.0.to_vec()),
                            l1_batch_number_of_initial_write: Some(
                                entry.l1_batch_number_of_initial_write.as_u32(),
                            ),
                            enumeration_index: Some(entry.enumeration_index),
                        };

                        chunk.storage_logs.push(pb);
                        header.l1_batch_number = entry.l1_batch_number_of_initial_write;
                    }
                } else {
                    has_more = false;
                }
            }

            // Ensure that write buffer has enough capacity.
            let chunk_len = chunk.encoded_len();
            if buf.capacity() < chunk_len {
                buf.reserve(chunk_len - buf.capacity());
            }

            chunk_index += 1;
            let path = PathBuf::new()
                .join(&self.basedir)
                .join(format!("{chunk_index}.gz"));

            header
                .storage_logs_chunks
                .push(types::SnapshotStorageLogsChunkMetadata {
                    chunk_id: chunk_index,
                    filepath: path
                        .clone()
                        .into_os_string()
                        .into_string()
                        .expect("path to string"),
                });

            // Serialize chunk.
            chunk.encode(&mut buf)?;

            let outfile = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;

            // Wrap in gzip compression before writing.
            let mut encoder = GzEncoder::new(outfile, Compression::default());
            encoder.write_all(&buf)?;
            encoder.finish()?;

            // Clear $tmp buffer.
            buf.truncate(0);
        }

        Ok(())
    }
}

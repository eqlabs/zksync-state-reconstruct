use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

mod bytecode;
mod database;
mod types;

use async_trait::async_trait;
use blake2::{Blake2s256, Digest};
use bytes::BytesMut;
use deflate::deflate_bytes_gzip;
use ethers::types::{Address, H256, U256, U64};
use eyre::Result;
use prost::Message;
use state_reconstruct_fetcher::{
    constants::{ethereum, storage},
    types::CommitBlock,
};
use tokio::sync::mpsc;

use self::{
    database::SnapshotDB,
    types::{SnapshotFactoryDependency, SnapshotHeader, SnapshotStorageLog},
};
use super::Processor;
use crate::processor::snapshot::types::MiniblockNumber;

pub mod protobuf {
    include!(concat!(env!("OUT_DIR"), "/protobuf.rs"));
}

pub const DEFAULT_DB_PATH: &str = "snapshot_db";

pub struct SnapshotBuilder {
    database: SnapshotDB,
}

impl SnapshotBuilder {
    pub fn new(db_path: Option<String>) -> Self {
        let db_path = match db_path {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_DB_PATH),
        };

        let mut database = SnapshotDB::new(db_path).unwrap();

        let idx = database
            .get_last_repeated_key_index()
            .expect("failed to read last repeated key index");

        // When last repeated key index is 0, there is no content in the DB.
        // Every write of new storage log key increases the index by one.
        if idx == 0 {
            reconstruct_genesis_state(&mut database, storage::INITAL_STATE_PATH).unwrap();
        }

        Self { database }
    }
}

#[async_trait]
impl Processor for SnapshotBuilder {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlock>) {
        while let Some(block) = rx.recv().await {
            // Initial calldata.
            for (key, value) in &block.initial_storage_changes {
                self.database
                    .insert_storage_log(&mut SnapshotStorageLog {
                        key: U256::from_little_endian(key),
                        value: self.database.process_value(U256::from(key), *value),
                        miniblock_number_of_initial_write: U64::from(0),
                        l1_batch_number_of_initial_write: U64::from(
                            block.l1_block_number.unwrap_or(0),
                        ),
                        enumeration_index: 0,
                    })
                    .expect("failed to insert storage_log_entry");
            }

            // Repeated calldata.
            for (index, value) in &block.repeated_storage_changes {
                let index = usize::try_from(*index).expect("truncation failed");
                let key = self
                    .database
                    .get_key_from_index(index as u64)
                    .expect("missing key");
                let value = self.database.process_value(U256::from(&key[0..32]), *value);

                if self
                    .database
                    .update_storage_log_value(index as u64, &value.to_fixed_bytes())
                    .is_err()
                {
                    let max_idx = self
                        .database
                        .get_last_repeated_key_index()
                        .expect("failed to get latest repeated key index");
                    tracing::error!(
                        "failed to find key with index {}, last repeated key index: {}",
                        index,
                        max_idx
                    );
                };
            }

            // Factory dependencies.
            for dep in block.factory_deps {
                self.database
                    .insert_factory_dep(&SnapshotFactoryDependency {
                        bytecode_hash: bytecode::hash_bytecode(&dep),
                        bytecode: dep,
                    })
                    .expect("failed to save factory dep");
            }
        }
    }
}

// TODO: Can this be made somewhat generic?
/// Attempts to reconstruct the genesis state from a CSV file.
fn reconstruct_genesis_state(database: &mut SnapshotDB, path: &str) -> Result<()> {
    fn cleanup_encoding(input: &'_ str) -> &'_ str {
        input
            .strip_prefix("E'\\\\x")
            .unwrap()
            .strip_suffix('\'')
            .unwrap()
    }

    let mut block_batched_accesses = vec![];

    let input = fs::read_to_string(path)?;
    for line in input.lines() {
        let mut separated = line.split(',');
        let _derived_key = separated.next().unwrap();
        let address = separated.next().unwrap();
        let key = separated.next().unwrap();
        let value = separated.next().unwrap();
        let op_number: u32 = separated.next().unwrap().parse()?;
        let _ = separated.next().unwrap();
        let miniblock_number: u32 = separated.next().unwrap().parse()?;

        if miniblock_number != 0 {
            break;
        }

        let address = Address::from_str(cleanup_encoding(address))?;
        let key = U256::from_str_radix(cleanup_encoding(key), 16)?;
        let value = U256::from_str_radix(cleanup_encoding(value), 16)?;

        let record = (address, key, value, op_number, miniblock_number);
        block_batched_accesses.push(record);
    }

    // Sort in block block.
    block_batched_accesses.sort_by(|a, b| match a.0.cmp(&b.0) {
        std::cmp::Ordering::Equal => match a.1.cmp(&b.1) {
            std::cmp::Ordering::Equal => match a.3.cmp(&b.3) {
                std::cmp::Ordering::Equal => {
                    panic!("must be unique")
                }
                a => a,
            },
            a => a,
        },
        a => a,
    });

    let mut key_set = std::collections::HashSet::new();

    // Batch.
    for el in &block_batched_accesses {
        let derived_key = derive_final_address_for_params(&el.0, &el.1);
        key_set.insert(derived_key);
    }

    let mut batched = vec![];
    let mut it = block_batched_accesses.into_iter();
    let mut previous = it.next().unwrap();
    for el in it {
        if el.0 != previous.0 || el.1 != previous.1 {
            batched.push((previous.0, previous.1, previous.2, previous.4));
        }

        previous = el;
    }

    // Finalize.
    batched.push((previous.0, previous.1, previous.2, previous.4));

    tracing::trace!("Have {} unique keys in the tree", key_set.len());

    for (address, key, value, miniblock_number) in batched {
        let derived_key = derive_final_address_for_params(&address, &key);
        let mut tmp = [0u8; 32];
        value.to_big_endian(&mut tmp);

        let key = U256::from_little_endian(&derived_key);
        let value = H256::from(tmp);

        if database.get_storage_log(&derived_key)?.is_none() {
            database.insert_storage_log(&mut SnapshotStorageLog {
                key,
                value,
                miniblock_number_of_initial_write: MiniblockNumber::from(miniblock_number),
                l1_batch_number_of_initial_write: U64::from(ethereum::GENESIS_BLOCK),
                enumeration_index: 0,
            })?;
        } else {
            database.update_storage_log_entry(&derived_key, value.as_bytes())?;
        }
    }

    Ok(())
}

fn derive_final_address_for_params(address: &Address, key: &U256) -> [u8; 32] {
    let mut buffer = [0u8; 64];
    buffer[12..32].copy_from_slice(&address.0);
    key.to_big_endian(&mut buffer[32..64]);

    let mut result = [0u8; 32];
    result.copy_from_slice(Blake2s256::digest(buffer).as_slice());

    result
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

        let path = PathBuf::new()
            .join(&self.basedir)
            .join("snapshot-header.json");

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
            let factory_dep: SnapshotFactoryDependency = bincode::deserialize(bs.as_ref())?;
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

        let path = PathBuf::new().join(&self.basedir).join("factory_deps.dat");
        header.factory_deps_filepath = path
            .clone()
            .into_os_string()
            .into_string()
            .expect("path to string");

        let mut outfile = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        // Serialize chunk.
        factory_deps.encode(&mut buf)?;

        // Wrap in gzip compression before writing.
        let compressed_buf = deflate_bytes_gzip(&buf);
        outfile.write_all(&compressed_buf)?;
        outfile.flush()?;

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

            let mut outfile = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;

            // Serialize chunk.
            chunk.encode(&mut buf)?;

            // Wrap in gzip compression before writing.
            let compressed_buf = deflate_bytes_gzip(&buf);
            outfile.write_all(&compressed_buf)?;
            outfile.flush()?;

            // Clear $tmp buffer.
            buf.truncate(0);
        }

        Ok(())
    }
}

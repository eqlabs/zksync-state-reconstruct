use std::{fs, path::PathBuf, str::FromStr};

pub mod database;
pub mod exporter;
pub mod importer;

mod bytecode;
mod types;

use async_trait::async_trait;
use blake2::{Blake2s256, Digest};
use ethers::types::{Address, H256, U256, U64};
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::{ethereum, storage},
    types::CommitBlock,
};
use tokio::sync::mpsc;

use self::{
    database::SnapshotDB,
    types::{SnapshotFactoryDependency, SnapshotStorageLog},
};
use super::Processor;
use crate::processor::snapshot::types::MiniblockNumber;

pub const DEFAULT_DB_PATH: &str = "snapshot_db";
pub const SNAPSHOT_HEADER_FILE_NAME: &str = "snapshot-header.json";
pub const SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX: &str = "factory_deps.proto.gzip";

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

    // Gets the next L1 batch number to be processed for ues in state recovery.
    pub fn get_last_l1_batch_number(&self) -> Result<Option<u64>> {
        self.database.get_last_l1_batch_number()
    }
}

#[async_trait]
impl Processor for SnapshotBuilder {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlock>) {
        while let Some(block) = rx.recv().await {
            // Initial calldata.
            for (key, value) in &block.initial_storage_changes {
                let key = U256::from_little_endian(key);
                let value = self
                    .database
                    .process_value(key, *value)
                    .expect("failed to get key from database");

                self.database
                    .insert_storage_log(&mut SnapshotStorageLog {
                        key,
                        value,
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
                let value = self
                    .database
                    .process_value(U256::from_big_endian(&key[0..32]), *value)
                    .expect("failed to get key from database");

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

            if let Some(number) = block.l1_block_number {
                let _ = self.database.set_last_l1_batch_number(number);
            };
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

#[cfg(test)]
mod tests {
    use std::fs;

    use indexmap::IndexMap;
    use state_reconstruct_fetcher::types::PackingType;

    use super::*;

    #[tokio::test]
    async fn simple() {
        let db_dir = "./test_db".to_string();
        let _ = fs::remove_dir_all(db_dir.clone());

        {
            let builder = SnapshotBuilder::new(Some(db_dir.clone()));
            let (tx, rx) = mpsc::channel::<CommitBlock>(5);

            tokio::spawn(async move {
                let key = U256::from_dec_str("1234").unwrap();
                let mut key1 = [0u8; 32];
                key.to_little_endian(&mut key1);
                let val1 = U256::from_dec_str("5678").unwrap();
                let mut initial_storage_changes = IndexMap::new();
                initial_storage_changes.insert(key1, PackingType::NoCompression(val1));
                let repeated_storage_changes = IndexMap::new();
                let cb = CommitBlock {
                    l1_block_number: Some(1),
                    l2_block_number: 2,
                    index_repeated_storage_changes: 0,
                    new_state_root: Vec::new(),
                    initial_storage_changes,
                    repeated_storage_changes,
                    factory_deps: Vec::new(),
                };
                tx.send(cb).await.unwrap();
            });

            builder.run(rx).await;
        }

        let db = SnapshotDB::new(PathBuf::from(db_dir.clone())).unwrap();

        let key = U256::from_dec_str("1234").unwrap();
        let mut key1 = [0u8; 32];
        key.to_big_endian(&mut key1);
        let Some(sl1) = db.get_storage_log(&key1).unwrap() else {
            panic!("key1 not found")
        };

        assert_eq!(sl1.key, key1.into());

        fs::remove_dir_all(db_dir).unwrap();
    }
}

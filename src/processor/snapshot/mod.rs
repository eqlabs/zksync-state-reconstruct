use std::{fs, path::PathBuf, str::FromStr};

use state_reconstruct_storage::snapshot::SnapshotDatabase;

pub mod exporter;
pub mod importer;

use async_trait::async_trait;
use ethers::types::{Address, H256, U256, U64};
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::{ethereum, storage, zksync::L2_BLOCK_NUMBER_ADDRESSES},
    types::CommitBlock,
};
use state_reconstruct_storage::{
    bytecode,
    types::{SnapshotFactoryDependency, SnapshotStorageLog},
};
use state_reconstruct_utils::{derive_final_address_for_params, h256_to_u256, unpack_block_info};
use tokio::sync::mpsc;

use super::Processor;

pub const DEFAULT_DB_PATH: &str = "snapshot_db";
pub const SNAPSHOT_HEADER_FILE_NAME: &str = "snapshot-header.json";
pub const SNAPSHOT_FACTORY_DEPS_FILE_NAME_SUFFIX: &str = "factory_deps.proto.gzip";

pub struct SnapshotBuilder {
    database: SnapshotDatabase,
}

impl SnapshotBuilder {
    pub fn new(db_path: Option<String>) -> Self {
        let db_path = match db_path {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_DB_PATH),
        };

        let mut database = SnapshotDatabase::new(db_path).unwrap();

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

    // Gets the next L1 block number to be processed for ues in state recovery.
    pub fn get_latest_l1_block_number(&self) -> Result<U64> {
        self.database
            .get_latest_l1_block_number()
            .map(|o| o.unwrap_or(U64::from(0)))
    }
}

#[async_trait]
impl Processor for SnapshotBuilder {
    async fn run(mut self, mut rx: mpsc::Receiver<CommitBlock>) {
        while let Some(block) = rx.recv().await {
            // Initial calldata.
            for (key, value) in &block.initial_storage_changes {
                let value = self
                    .database
                    .process_value(*key, *value)
                    .expect("failed to get key from database");

                self.database
                    .insert_storage_log(&mut SnapshotStorageLog {
                        key: *key,
                        value,
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

                // We make sure to track writes to the L2 block number address.
                let hex_key = hex::encode(key);
                if L2_BLOCK_NUMBER_ADDRESSES.contains(&hex_key.as_ref()) {
                    let (block_number, _timestamp) = unpack_block_info(h256_to_u256(value));
                    self.database
                        .set_latest_l2_block_number(block_number)
                        .expect("failed to insert latest l2 block number");
                }

                if self
                    .database
                    .update_storage_log_value(index as u64, value)
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

            let _ = self
                .database
                .set_latest_l1_batch_number(block.l1_batch_number);

            if let Some(number) = block.l1_block_number {
                let _ = self.database.set_latest_l1_block_number(number);
            };
        }
    }
}

// TODO: Can this be made somewhat generic?
/// Attempts to reconstruct the genesis state from a CSV file.
fn reconstruct_genesis_state(database: &mut SnapshotDatabase, path: &str) -> Result<()> {
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

    for (address, key, value, _miniblock_number) in batched {
        let derived_key = derive_final_address_for_params(&address, &key);
        let mut tmp = [0u8; 32];
        value.to_big_endian(&mut tmp);

        let key = U256::from_little_endian(&derived_key);
        let value = H256::from(tmp);

        database.insert_storage_log(&mut SnapshotStorageLog {
            key,
            value,
            l1_batch_number_of_initial_write: U64::from(ethereum::GENESIS_BLOCK),
            enumeration_index: 0,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use indexmap::IndexMap;
    use state_reconstruct_storage::PackingType;

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
                let val = U256::from_dec_str("5678").unwrap();
                let mut initial_storage_changes = IndexMap::new();
                initial_storage_changes.insert(key, PackingType::NoCompression(val));

                let repeated_storage_changes = IndexMap::new();
                let cb = CommitBlock {
                    l1_block_number: Some(1),
                    l1_batch_number: 2,
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

        let db = SnapshotDatabase::new(PathBuf::from(db_dir.clone())).unwrap();

        let key = U256::from_dec_str("1234").unwrap();
        let Some(log) = db.get_storage_log(&key).unwrap() else {
            panic!("key not found")
        };

        assert_eq!(log.key, key);

        fs::remove_dir_all(db_dir).unwrap();
    }
}

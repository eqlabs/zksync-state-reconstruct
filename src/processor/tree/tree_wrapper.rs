use std::{collections::HashMap, fs, num::NonZeroU32, path::Path, str::FromStr, sync::Arc};

use ethers::types::{Address, H256, U256, U64};
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::storage::{INITAL_STATE_PATH, INNER_DB_NAME},
    types::CommitBlock,
};
use state_reconstruct_storage::{
    reconstruction::ReconstructionDatabase, types::SnapshotStorageLogsChunk, PackingType,
};
use state_reconstruct_utils::derive_final_address_for_params;
use thiserror::Error;
use tokio::sync::{
    mpsc::{self, Receiver},
    Mutex,
};
use zksync_merkle_tree::{Database, MerkleTree, RocksDBWrapper, TreeEntry};
use zksync_storage::{RocksDB, RocksDBOptions};

use super::RootHash;

#[derive(Error, Debug)]
pub enum TreeError {
    #[error("block mismatch")]
    BlockMismatch,
}

pub struct TreeWrapper {
    index_to_key: HashMap<u64, U256>,
    key_to_value: HashMap<U256, H256>,
    tree: MerkleTree<RocksDBWrapper>,
    inner_db: Arc<Mutex<ReconstructionDatabase>>,
}

impl TreeWrapper {
    /// Attempts to create a new [`TreeWrapper`].
    pub async fn new(
        db_path: &Path,
        inner_db: Arc<Mutex<ReconstructionDatabase>>,
        reconstruct: bool,
    ) -> Result<Self> {
        let db_opt = RocksDBOptions {
            max_open_files: NonZeroU32::new(2048),
            ..Default::default()
        };
        let db = RocksDBWrapper::from(RocksDB::with_options(db_path, db_opt)?);
        let mut tree = MerkleTree::new(db);

        if reconstruct {
            let mut guard = inner_db.lock().await;
            reconstruct_genesis_state(&mut tree, &mut guard, INITAL_STATE_PATH)?;
        }

        Ok(Self {
            index_to_key: HashMap::new(),
            key_to_value: HashMap::new(),
            tree,
            inner_db,
        })
    }

    pub async fn new_snapshot_wrapper(db_path: &Path) -> Result<Self> {
        let inner_db_path = db_path.join(INNER_DB_NAME);
        let new_state = ReconstructionDatabase::new(inner_db_path.clone())?;
        let snapshot = Arc::new(Mutex::new(new_state));
        Self::new(db_path, snapshot.clone(), true).await
    }

    /// Inserts a block into the tree and returns the root hash of the resulting state tree.
    pub async fn insert_block(&mut self, block: &CommitBlock) -> Result<RootHash> {
        self.clear_known_base();
        let mut tree_entries = Vec::with_capacity(block.initial_storage_changes.len());
        // INITIAL CALLDATA.
        let mut index =
            block.index_repeated_storage_changes - (block.initial_storage_changes.len() as u64);
        for (key, value) in &block.initial_storage_changes {
            self.insert_known_key(index, *key);
            let value = self.process_value(*key, *value);

            tree_entries.push(TreeEntry::new(*key, index, value));
            self.inner_db
                .lock()
                .await
                .add_key(key)
                .expect("cannot add key");
            index += 1;
        }

        // REPEATED CALLDATA.
        for (index, value) in &block.repeated_storage_changes {
            let index = *index;
            // Index is 1-based so we subtract 1.
            let key = self
                .inner_db
                .lock()
                .await
                .get_key(index - 1)
                .expect("invalid key index");
            self.insert_known_key(index, key);
            let value = self.process_value(key, *value);

            tree_entries.push(TreeEntry::new(key, index, value));
        }

        let output = self.tree.extend(tree_entries);
        let root_hash = output.root_hash;

        tracing::debug!(
            "Root hash of batch {} = {}",
            block.l1_batch_number,
            hex::encode(root_hash)
        );

        let root_hash_bytes = root_hash.as_bytes();
        if root_hash_bytes == block.new_state_root {
            tracing::debug!("Successfully processed batch {}", block.l1_batch_number);

            Ok(root_hash)
        } else {
            tracing::error!(
                "Root hash mismatch!\nLocal: {}\nPublished: {}",
                hex::encode(root_hash_bytes),
                hex::encode(&block.new_state_root)
            );
            let mut rollback_entries = Vec::with_capacity(self.index_to_key.len());
            for (index, key) in &self.index_to_key {
                let value = self.key_to_value.get(key).unwrap();
                rollback_entries.push(TreeEntry::new(*key, *index, *value));
            }

            self.tree.extend(rollback_entries);
            Err(TreeError::BlockMismatch.into())
        }
    }

    pub async fn restore_from_snapshot(
        &mut self,
        mut rx: Receiver<SnapshotStorageLogsChunk>,
        l1_batch_number: U64,
    ) -> Result<()> {
        let mut total_tree_entries = 0;

        let (tree_tx, mut tree_rx) = mpsc::channel(1);
        tokio::spawn({
            let inner_db = self.inner_db.clone();
            async move {
                let mut inner_db = inner_db.lock().await;
                while let Some(chunk) = rx.recv().await {
                    let mut tree_entries = Vec::new();

                    for log in &chunk.storage_logs {
                        tree_entries.push(TreeEntry::new(
                            log.key,
                            log.enumeration_index,
                            log.value,
                        ));
                        inner_db.add_key(&log.key).expect("cannot add key");
                    }

                    tree_tx.send(tree_entries).await.expect("receiver dropped");
                }
            }
        });

        let mut i = 0;
        while let Some(tree_entries) = tree_rx.recv().await {
            total_tree_entries += tree_entries.len();
            self.tree.extend(tree_entries);

            tracing::info!("Chunk {} was successfully imported!", i + 1);
            i += 1;
        }

        tracing::info!(
            "Successfully imported snapshot containing {total_tree_entries} storage logs!",
        );

        let root_hash = hex::encode(self.tree.latest_root_hash());
        tracing::debug!("Current root hash is: {}", root_hash);

        self.inner_db
            .lock()
            .await
            .set_latest_l1_batch_number(l1_batch_number.as_u64())?;

        Ok(())
    }

    fn process_value(&mut self, key: U256, value: PackingType) -> H256 {
        let version = self.tree.latest_version().unwrap_or_default();
        if let Ok(leaf) = self.tree.entries(version, &[key]) {
            let hash = leaf.last().unwrap().value;
            self.insert_known_value(key, hash);
            let existing_value = U256::from(hash.to_fixed_bytes());
            // NOTE: We're explicitly allowing over-/underflow as per the spec.
            let processed_value = match value {
                PackingType::NoCompression(v) | PackingType::Transform(v) => v,
                PackingType::Add(v) => existing_value.overflowing_add(v).0,
                PackingType::Sub(v) => existing_value.overflowing_sub(v).0,
            };
            let mut buffer = [0; 32];
            processed_value.to_big_endian(&mut buffer);
            H256::from(buffer)
        } else {
            panic!("no key found for version")
        }
    }

    fn clear_known_base(&mut self) {
        self.index_to_key.clear();
        self.key_to_value.clear();
    }

    fn insert_known_key(&mut self, index: u64, key: U256) {
        if let Some(old_key) = self.index_to_key.insert(index, key) {
            assert_eq!(old_key, key);
        }
    }

    fn insert_known_value(&mut self, key: U256, value: H256) {
        if let Some(old_value) = self.key_to_value.insert(key, value) {
            tracing::debug!(
                "Updated value at {:?} from {:?} to {:?}",
                key,
                old_value,
                value
            );
        }
    }
}

/// Attempts to reconstruct the genesis state from a CSV file.
fn reconstruct_genesis_state<D: Database>(
    tree: &mut MerkleTree<D>,
    snapshot: &mut ReconstructionDatabase,
    path: &str,
) -> Result<()> {
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

        let record = (address, key, value, op_number);
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
            batched.push((previous.0, previous.1, previous.2));
        }

        previous = el;
    }

    // Finalize.
    batched.push((previous.0, previous.1, previous.2));

    tracing::trace!("Have {} unique keys in the tree", key_set.len());

    let mut tree_entries = Vec::with_capacity(batched.len());
    let mut index = 1;
    for (address, key, value) in batched {
        let derived_key = derive_final_address_for_params(&address, &key);
        let mut tmp = [0u8; 32];
        value.to_big_endian(&mut tmp);

        let key = U256::from_little_endian(&derived_key);
        let value = H256::from(tmp);
        tree_entries.push(TreeEntry::new(key, index, value));
        snapshot.add_key(&key).expect("cannot add genesis key");
        index += 1;
    }

    let output = tree.extend(tree_entries);
    tracing::trace!("Initial state root = {}", hex::encode(output.root_hash));

    Ok(())
}

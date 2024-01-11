use std::{fs, path::Path, str::FromStr, sync::Arc};

use blake2::{Blake2s256, Digest};
use ethers::types::{Address, H256, U256};
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::storage::INITAL_STATE_PATH, database::InnerDB, types::CommitBlock,
};
use tokio::sync::Mutex;
use zksync_merkle_tree::{Database, MerkleTree, RocksDBWrapper};

use super::RootHash;

pub struct TreeWrapper {
    tree: MerkleTree<RocksDBWrapper>,
    snapshot: Arc<Mutex<InnerDB>>,
}

impl TreeWrapper {
    /// Attempts to create a new [`TreeWrapper`].
    pub async fn new(
        db_path: &Path,
        snapshot: Arc<Mutex<InnerDB>>,
        reconstruct: bool,
    ) -> Result<Self> {
        let db = RocksDBWrapper::new(db_path);
        let mut tree = MerkleTree::new(db);

        if reconstruct {
            let mut guard = snapshot.lock().await;
            reconstruct_genesis_state(&mut tree, &mut *guard, INITAL_STATE_PATH)?;
        }

        Ok(Self { tree, snapshot })
    }

    /// Inserts a block into the tree and returns the root hash of the resulting state tree.
    pub async fn insert_block(&mut self, block: &CommitBlock) -> RootHash {
        // INITIAL CALLDATA.
        let mut key_value_pairs: Vec<(U256, H256)> =
            Vec::with_capacity(block.initial_storage_changes.len());
        for (key, value) in &block.initial_storage_changes {
            let key = U256::from_little_endian(key);
            let value = H256::from(value);

            key_value_pairs.push((key, value));
            self.snapshot.lock().await.add_key(&key).expect("cannot add key");
        }

        // REPEATED CALLDATA.
        for (index, value) in &block.repeated_storage_changes {
            let index = u64::try_from(*index).expect("truncation failed");
            // Index is 1-based so we subtract 1.
            let key = self.snapshot.lock().await.get_key(index - 1).unwrap();
            let value = H256::from(value);

            key_value_pairs.push((key, value));
        }

        let output = self.tree.extend(key_value_pairs);
        let root_hash = output.root_hash;

        assert_eq!(root_hash.as_bytes(), block.new_state_root);
        tracing::debug!(
            "Root hash of block {} = {}",
            block.l2_block_number,
            hex::encode(root_hash)
        );

        tracing::debug!("Successfully processed block {}", block.l2_block_number);

        root_hash
    }
}

/// Attempts to reconstruct the genesis state from a CSV file.
fn reconstruct_genesis_state<D: Database>(
    tree: &mut MerkleTree<D>,
    snapshot: &mut InnerDB,
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

    let mut key_value_pairs: Vec<(U256, H256)> = Vec::with_capacity(batched.len());
    for (address, key, value) in batched {
        let derived_key = derive_final_address_for_params(&address, &key);
        // TODO: what to do here?
        // let version = tree.latest_version().unwrap_or_default();
        // let _leaf = tree.read_leaves(version, &[key]);

        // let existing_value = U256::from_big_endian(existing_leaf.leaf.value());
        // if existing_value == value {
        //     // we downgrade to read
        //     // println!("Downgrading to read")
        // } else {
        // we write
        let mut tmp = [0u8; 32];
        value.to_big_endian(&mut tmp);

        let key = U256::from_little_endian(&derived_key);
        let value = H256::from(tmp);
        key_value_pairs.push((key, value));
        snapshot.add_key(&key).expect("cannot add genesis key");
    }

    let output = tree.extend(key_value_pairs);
    tracing::trace!("Initial state root = {}", hex::encode(output.root_hash));

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

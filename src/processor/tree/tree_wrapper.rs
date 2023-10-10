use std::{fs, path::Path, str::FromStr};

use ethers::types::{Address, H256, U256};
use eyre::Result;
use indexmap::IndexSet;
use zk_evm::aux_structures::LogQuery;
use zksync_merkle_tree::{Database, MerkleTree, RocksDBWrapper};

use crate::{constants::storage::INITAL_STATE_PATH, CommitBlockInfoV1};

pub type RootHash = H256;

pub struct TreeWrapper<'a> {
    tree: MerkleTree<'a, RocksDBWrapper>,
    pub index_to_key_map: IndexSet<U256>,
}

impl TreeWrapper<'static> {
    /// Attempts to create a new [`TreeWrapper`].
    pub fn new(db_path: &Path, mut index_to_key_map: IndexSet<U256>) -> Result<Self> {
        let db = RocksDBWrapper::new(db_path);
        let mut tree = MerkleTree::new(db);

        // If an existing `index_to_key` mapping was supplied, use that.
        // Otherwise, reconstruct the genesis state.
        if index_to_key_map.is_empty() {
            println!("was empty!");
            reconstruct_genesis_state(&mut tree, &mut index_to_key_map, INITAL_STATE_PATH)?;
        }

        Ok(Self {
            tree,
            index_to_key_map,
        })
    }

    /// Inserts a block into the tree and returns the root hash of the resulting state tree.
    pub fn insert_block(&mut self, block: &CommitBlockInfoV1) -> RootHash {
        // INITIAL CALLDATA.
        let mut key_value_pairs: Vec<(U256, H256)> =
            Vec::with_capacity(block.initial_storage_changes.len());
        for (key, value) in &block.initial_storage_changes {
            let key = U256::from_little_endian(key);
            let value = H256::from(value);

            key_value_pairs.push((key, value));
            self.index_to_key_map.insert(key);
        }

        // REPEATED CALLDATA.
        for (index, value) in &block.repeated_storage_changes {
            let index = usize::try_from(*index).expect("truncation failed");
            // Index is 1-based so we subtract 1.
            let key = *self.index_to_key_map.get_index(index - 1).unwrap();
            let value = H256::from(value);

            key_value_pairs.push((key, value));
        }

        let output = self.tree.extend(key_value_pairs);
        let root_hash = output.root_hash;

        assert_eq!(root_hash.as_bytes(), block.new_state_root);
        println!(
            "Root hash of block {} = {}",
            block.block_number,
            hex::encode(root_hash)
        );

        root_hash
    }

    pub fn latest_root_hash(&self) -> RootHash {
        self.tree.latest_root_hash()
    }
}

/// Attempts to reconstruct the genesis state from a CSV file.
fn reconstruct_genesis_state<D: Database>(
    tree: &mut MerkleTree<D>,
    index_to_key: &mut IndexSet<U256>,
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
        let derived_key = LogQuery::derive_final_address_for_params(&el.0, &el.1);
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

    println!("Have {} unique keys in the tree", key_set.len());

    let mut key_value_pairs: Vec<(U256, H256)> = Vec::with_capacity(batched.len());
    for (address, key, value) in batched {
        let derived_key = LogQuery::derive_final_address_for_params(&address, &key);
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
        index_to_key.insert(key);
    }

    let output = tree.extend(key_value_pairs);
    dbg!(tree.latest_version());
    println!("Initial state root = {}", hex::encode(output.root_hash));

    Ok(())
}

use std::{fs, path::Path, str::FromStr, sync::Arc};

use blake2::{Blake2s256, Digest};
use ethers::types::{Address, H256, U256};
use eyre::Result;
use state_reconstruct_fetcher::{
    constants::storage::INITAL_STATE_PATH,
    database::InnerDB,
    types::{v2::PackingType, CommitBlock},
};
use tokio::sync::Mutex;
use zksync_merkle_tree::{domain::ZkSyncTree, RocksDBWrapper, TreeInstruction};

use super::RootHash;

pub struct TreeWrapper {
    tree: ZkSyncTree,
    snapshot: Arc<Mutex<InnerDB>>,
}

impl TreeWrapper {
    /// Attempts to create a new [`TreeWrapper`].
    pub async fn new(
        db_path: &Path,
        snapshot: Arc<Mutex<InnerDB>>,
        reconstruct: bool,
    ) -> Result<Self> {
        let db = RocksDBWrapper::new(db_path)?;
        let mut tree = ZkSyncTree::new_lightweight(db);

        if reconstruct {
            let mut guard = snapshot.lock().await;
            reconstruct_genesis_state(&mut tree, &mut guard, INITAL_STATE_PATH)?;
        }

        Ok(Self { tree, snapshot })
    }

    /// Inserts a block into the tree and returns the root hash of the resulting state tree.
    pub async fn insert_block(&mut self, block: &CommitBlock) -> RootHash {
        let mut instructions = Vec::with_capacity(block.initial_storage_changes.len());
        // INITIAL CALLDATA.
        let mut index =
            block.index_repeated_storage_changes - (block.initial_storage_changes.len() as u64);
        for (key, value) in &block.initial_storage_changes {
            let key = U256::from_little_endian(key);
            let value = self.process_value(key, *value);

            instructions.push(TreeInstruction::write(key, index, value));
            self.snapshot
                .lock()
                .await
                .add_key(&key)
                .expect("cannot add key");
            index += 1;
        }

        // REPEATED CALLDATA.
        for (index, value) in &block.repeated_storage_changes {
            let index = *index;
            // Index is 1-based so we subtract 1.
            let key = self
                .snapshot
                .lock()
                .await
                .get_key(index - 1)
                .expect("invalid key index");
            let value = self.process_value(key, *value);

            instructions.push(TreeInstruction::write(key, index, value));
        }

        let output = self.tree.process_l1_batch(instructions.as_slice());
        let root_hash = output.root_hash;

        tracing::debug!(
            "Root hash of block {} = {}",
            block.l2_block_number,
            hex::encode(root_hash)
        );

        assert_eq!(root_hash.as_bytes(), block.new_state_root);
        tracing::debug!("Successfully processed block {}", block.l2_block_number);

        root_hash
    }

    fn latest_version(&self) -> L1BatchNumber {
        let bn = self.tree.next_l1_batch_number();
        L1BatchNumber(bn.0 - 1)
    }

    fn process_value(&self, key: U256, value: PackingType) -> H256 {
        let processed_value = match value {
            PackingType::NoCompression(v) | PackingType::Transform(v) => v,
            PackingType::Add(_) | PackingType::Sub(_) => {
                let version = self.latest_version();
                if let Ok(leaf) = self.tree.entries_with_proofs(version, &[key]) {
                    let hash = leaf.last().unwrap().value;
                    let existing_value = U256::from(hash.to_fixed_bytes());
                    // NOTE: We're explicitly allowing over-/underflow as per the spec.
                    match value {
                        PackingType::Add(v) => existing_value.overflowing_add(v).0,
                        PackingType::Sub(v) => existing_value.overflowing_sub(v).0,
                        _ => unreachable!(),
                    }
                } else {
                    panic!("no key found for version")
                }
            }
        };

        let mut buffer = [0; 32];
        processed_value.to_big_endian(&mut buffer);
        H256::from(buffer)
    }
}

/// Attempts to reconstruct the genesis state from a CSV file.
fn reconstruct_genesis_state(
    tree: &mut ZkSyncTree,
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

    let mut instructions = Vec::with_capacity(batched.len());
    let mut index = 1;
    for (address, key, value) in batched {
        let derived_key = derive_final_address_for_params(&address, &key);
        let mut tmp = [0u8; 32];
        value.to_big_endian(&mut tmp);

        let key = U256::from_little_endian(&derived_key);
        let value = H256::from(tmp);
        instructions.push(TreeInstruction::write(key, index, value));
        snapshot.add_key(&key).expect("cannot add genesis key");
        index += 1;
    }

    let output = tree.process_l1_batch(instructions.as_slice());
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

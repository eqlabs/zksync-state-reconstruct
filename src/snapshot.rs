use std::{fs, io, path::Path};

use ethers::types::{U256, U64};
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

/// Struct containing the fields used for restoring the tree state.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct StateSnapshot {
    /// The latest l1 block number that was processed.
    pub latest_l1_block_number: U64,
    /// The latest l2 block number that was processed.
    pub latest_l2_block_number: u64,
    /// The mappings of index to key values.
    pub index_to_key_map: IndexSet<U256>,
}

impl StateSnapshot {
    /// Reads and serializes the json file containing the index to key mappings.
    pub fn read(path: &Path) -> Result<StateSnapshot, io::Error> {
        let contents = fs::read_to_string(path)?;
        let state: StateSnapshot = serde_json::from_str(&contents)?;

        Ok(state)
    }

    /// Writes the json file containing the index to key mappings.
    pub fn write(&self, path: &Path) -> Result<(), io::Error> {
        let content = serde_json::to_string_pretty(&self)?;
        fs::write(path, content)
    }
}

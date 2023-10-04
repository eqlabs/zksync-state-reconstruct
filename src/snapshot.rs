#![allow(dead_code)]
use std::{fs, io};

use ethers::types::U256;
use indexmap::IndexSet;
use serde::{Deserialize, Serialize};

use crate::constants::ethereum;

/// Struct containing the fields used for restoring the tree state.
#[derive(Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The latest l1 block number that was processed.
    pub current_l1_block_number: u64,
    /// The latest l2 block number that was processed.
    pub latest_l2_block_number: U256,
    /// The mappings of index to key values.
    pub index_to_key_map: IndexSet<U256>,
}

impl Default for StateSnapshot {
    fn default() -> Self {
        Self {
            current_l1_block_number: ethereum::GENESIS_BLOCK,
            latest_l2_block_number: U256::default(),
            index_to_key_map: IndexSet::default(),
        }
    }
}

impl StateSnapshot {
    pub fn new(
        latest_l1_block_number: u64,
        latest_l2_block_number: U256,
        index_to_key_map: IndexSet<U256>,
    ) -> Self {
        Self {
            current_l1_block_number: latest_l1_block_number,
            latest_l2_block_number,
            index_to_key_map,
        }
    }

    /// Reads and serializes the json file containing the index to key mappings.
    pub fn read(path: &str) -> Result<StateSnapshot, io::Error> {
        let contents = fs::read_to_string(path)?;
        let state: StateSnapshot = serde_json::from_str(&contents)?;

        Ok(state)
    }

    /// Writes the json file containing the index to key mappings.
    pub fn write(&self, path: &str) -> Result<(), io::Error> {
        let content = serde_json::to_string_pretty(&self)?;
        fs::write(path, content)
    }
}

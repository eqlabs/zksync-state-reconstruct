// FIXME:
#![allow(dead_code)]
use std::fmt;

use chrono::{offset::Utc, DateTime};
use ethers::types::{H256, U256, U64};
use serde::{Deserialize, Serialize};

pub type L1BatchNumber = U64;
pub type MiniblockNumber = U64;

pub type StorageKey = U256;
pub type StorageValue = H256;

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotHeader {
    pub l1_batch_number: L1BatchNumber,
    pub miniblock_number: MiniblockNumber,
    /// Chunk metadata ordered by chunk_id
    pub chunks: Vec<SnapshotChunkMetadata>,
    // TODO:
    // pub last_l1_batch_with_metadata: L1BatchWithMetadata,
    pub generated_at: DateTime<Utc>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotChunkMetadata {
    pub key: SnapshotStorageKey,
    /// Can be either a gs or filesystem path
    pub filepath: String,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotStorageKey {
    pub l1_batch_number: L1BatchNumber,
    /// Chunks with smaller id's must contain storage_logs with smaller hashed_keys
    pub chunk_id: u64,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotChunk {
    // Sorted by hashed_keys interpreted as little-endian numbers
    pub storage_logs: Vec<SnapshotStorageLog>,
    pub factory_deps: Vec<SnapshotFactoryDependency>,
}

// "most recent" for each key together with info when the key was first used
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotStorageLog {
    pub key: StorageKey,
    pub value: StorageValue,
    pub miniblock_number_of_initial_write: MiniblockNumber,
    pub l1_batch_number_of_initial_write: L1BatchNumber,
    pub enumeration_index: u64,
}

impl fmt::Display for SnapshotStorageLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{},{},{},{}",
            self.key,
            hex::encode(self.value),
            self.miniblock_number_of_initial_write,
            self.l1_batch_number_of_initial_write,
            self.enumeration_index
        )
    }
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotFactoryDependency {
    pub bytecode_hash: H256,
    pub bytecode: Vec<u8>,
}

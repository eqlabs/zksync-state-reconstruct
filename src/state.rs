use ethers::types::U256;
use std::collections::HashMap;
use std::vec::Vec;

/// Data needed to commit new block
pub struct CommitBlockInfoV1 {
    /// L2 block number.
    pub block_number: u64,
    /// Unix timestamp denoting the start of the block execution.
    pub timestamp: u64,
    /// The serial number of the shortcut index that's used as a unique identifier for storage keys that were used twice or more.
    pub index_repeated_storage_changes: u64,
    /// The state root of the full state tree.
    pub new_state_root: Vec<u8>,
    /// Number of priority operations to be processed.
    pub number_of_l1_txs: U256,
    /// The root hash of the tree that contains all L2 -> L1 logs in the block.
    pub l2_logs_tree_root: Vec<u8>,
    /// Hash of all priority operations from this block.
    pub priority_operations_hash: Vec<u8>,
    /// Storage write access as a concatenation key-value.
    pub initial_storage_changes: HashMap<[u8; 32], [u8; 32]>,
    /// Storage write access as a concatenation index-value.
    pub repeated_storage_changes: HashMap<u64, [u8; 32]>,
    /// Concatenation of all L2 -> L1 logs in the block.
    pub l2_logs: Vec<u8>,
    /// (contract bytecodes) array of L2 bytecodes that were deployed.
    pub factory_deps: Vec<Vec<u8>>,
}

pub enum L2ToL1Pubdata {
    L2ToL1Log,
    L2ToL2Message,
    PublishedBytecode,
    CompressedStateDiff,
}

/// Data needed to commit new block
pub struct CommitBlockInfoV2 {
    /// L2 block number.
    block_number: u64,
    /// Unix timestamp denoting the start of the block execution.
    timestamp: u64,
    /// The serial number of the shortcut index that's used as a unique identifier for storage keys that were used twice or more.
    index_repeated_storage_changes: u64,
    /// The state root of the full state tree.
    new_state_root: Vec<u8>,
    /// Number of priority operations to be processed.
    number_of_l1_txs: U256,
    /// Hash of all priority operations from this block.
    priority_operations_hash: Vec<u8>,
    /// Concatenation of all L2 -> L1 system logs in the block.
    system_logs: Vec<u8>,
    /// Total pubdata committed to as part of bootloader run. Contents are: l2Tol1Logs <> l2Tol1Messages <> publishedBytecodes <> stateDiffs.
    total_l2_to_l1_pubdata: Vec<L2ToL1Pubdata>,
}

impl CommitBlockInfoV1 {
    pub fn as_v2(&self) -> CommitBlockInfoV2 {
        CommitBlockInfoV2 {
            block_number: self.block_number,
            timestamp: self.timestamp,
            index_repeated_storage_changes: self.index_repeated_storage_changes,
            new_state_root: self.new_state_root.clone(),
            number_of_l1_txs: self.number_of_l1_txs,
            priority_operations_hash: self.priority_operations_hash.to_vec(),
            system_logs: vec![],
            total_l2_to_l1_pubdata: vec![],
        }
    }
}

use ethers::types::U256;
use std::collections::HashMap;
use std::vec::Vec;

/// @notice Data needed to commit new block
/// @param blockNumber Number of the committed block
/// @param timestamp Unix timestamp denoting the start of the block execution
/// @param indexRepeatedStorageChanges The serial number of the shortcut index that's used as a unique identifier for storage keys that were used twice or more
/// @param newStateRoot The state root of the full state tree
/// @param numberOfLayer1Txs Number of priority operations to be processed
/// @param l2LogsTreeRoot The root hash of the tree that contains all L2 -> L1 logs in the block
/// @param priorityOperationsHash Hash of all priority operations from this block
/// @param initialStorageChanges Storage write access as a concatenation key-value
/// @param repeatedStorageChanges Storage write access as a concatenation index-value
/// @param l2Logs concatenation of all L2 -> L1 logs in the block
/// @param l2ArbitraryLengthMessages array of hash preimages that were sent as value of L2 logs by special system L2 contract
/// @param factoryDeps (contract bytecodes) array of l2 bytecodes that were deployed
pub struct CommitBlockInfoV1 {
    pub block_number: u64,
    pub timestamp: u64,
    pub index_repeated_storage_changes: u64,
    pub new_state_root: Vec<u8>,
    pub number_of_l1_txs: U256,
    pub l2_logs_tree_root: Vec<u8>,
    pub priority_operations_hash: Vec<u8>,
    pub initial_storage_changes: HashMap<[u8; 32], [u8; 32]>,
    pub repeated_storage_changes: HashMap<u64, [u8; 32]>,
    pub l2_logs: Vec<u8>,
    pub factory_deps: Vec<Vec<u8>>,
}

pub enum L2ToL1Pubdata {
    L2ToL1Log,
    L2ToL2Message,
    PublishedBytecode,
    CompressedStateDiff,
}

/// @notice Data needed to commit new block
/// @param blockNumber Number of the committed block
/// @param timestamp Unix timestamp denoting the start of the block execution
/// @param indexRepeatedStorageChanges The serial number of the shortcut index that's used as a unique identifier for storage keys that were used twice or more
/// @param newStateRoot The state root of the full state tree
/// @param numberOfLayer1Txs Number of priority operations to be processed
/// @param priorityOperationsHash Hash of all priority operations from this block
/// @param systemLogs concatenation of all L2 -> L1 system logs in the block
/// @param totalL2ToL1Pubdata Total pubdata committed to as part of bootloader run. Contents are: l2Tol1Logs <> l2Tol1Messages <> publishedBytecodes <> stateDiffs
pub struct CommitBlockInfoV2 {
    block_number: u64,
    timestamp: u64,
    index_repeated_storage_changes: u64,
    new_state_root: Vec<u8>,
    number_of_l1_txs: U256,
    priority_operations_hash: Vec<u8>,
    system_logs: Vec<u8>,
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

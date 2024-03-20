use ethers::{abi, types::U256};
use serde::{Deserialize, Serialize};

use super::{
    common::{parse_resolved_pubdata, ExtractedToken},
    CommitBlockFormat, CommitBlockInfo, L2ToL1Pubdata, ParseError,
};

/// Data needed to commit new block
#[derive(Debug, Serialize, Deserialize)]
pub struct V2 {
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
    /// Hash of all priority operations from this block.
    pub priority_operations_hash: Vec<u8>,
    /// Concatenation of all L2 -> L1 system logs in the block.
    pub system_logs: Vec<u8>,
    /// Total pubdata committed to as part of bootloader run. Contents are: l2Tol1Logs <> l2Tol1Messages <> publishedBytecodes <> stateDiffs.
    pub total_l2_to_l1_pubdata: Vec<L2ToL1Pubdata>,
}

impl CommitBlockFormat for V2 {
    fn to_enum_variant(self) -> CommitBlockInfo {
        CommitBlockInfo::V2(self)
    }
}

impl TryFrom<&abi::Token> for V2 {
    type Error = ParseError;

    /// Try to parse Ethereum ABI token.
    ///
    /// * `token` - ABI token of `CommitBlockInfo` type on Ethereum.
    fn try_from(token: &abi::Token) -> Result<Self, Self::Error> {
        let ExtractedToken {
            new_l2_block_number,
            timestamp,
            new_enumeration_index,
            state_root,
            number_of_l1_txs,
            priority_operations_hash,
            system_logs,
            total_l2_to_l1_pubdata,
        } = token.try_into()?;
        let new_enumeration_index = new_enumeration_index.as_u64();

        let total_l2_to_l1_pubdata = parse_resolved_pubdata(&total_l2_to_l1_pubdata)?;
        let blk = V2 {
            block_number: new_l2_block_number.as_u64(),
            timestamp: timestamp.as_u64(),
            index_repeated_storage_changes: new_enumeration_index,
            new_state_root: state_root,
            number_of_l1_txs,
            priority_operations_hash,
            system_logs,
            total_l2_to_l1_pubdata,
        };

        Ok(blk)
    }
}

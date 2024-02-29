use ethers::{abi, types::U256};
use serde::{Deserialize, Serialize};

use super::{
    common::{parse_compressed_state_diffs, read_next_n_bytes, ExtractedToken},
    CommitBlockFormat, CommitBlockInfo, L2ToL1Pubdata, ParseError,
};
use crate::constants::zksync::L2_TO_L1_LOG_SERIALIZE_SIZE;

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

    /// Try to parse Ethereum ABI token into [`CommitBlockInfo`].
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

        let total_l2_to_l1_pubdata = parse_total_l2_to_l1_pubdata(total_l2_to_l1_pubdata)?;
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

fn parse_total_l2_to_l1_pubdata(bytes: Vec<u8>) -> Result<Vec<L2ToL1Pubdata>, ParseError> {
    let mut l2_to_l1_pubdata = Vec::new();
    let mut pointer = 0;

    // Skip over logs and messages.
    let num_of_l1_to_l2_logs = u32::from_be_bytes(read_next_n_bytes(&bytes, &mut pointer));
    pointer += L2_TO_L1_LOG_SERIALIZE_SIZE * num_of_l1_to_l2_logs as usize;

    let num_of_messages = u32::from_be_bytes(read_next_n_bytes(&bytes, &mut pointer));
    for _ in 0..num_of_messages {
        let current_message_len = u32::from_be_bytes(read_next_n_bytes(&bytes, &mut pointer));
        pointer += current_message_len as usize;
    }

    // Parse published bytecodes.
    let num_of_bytecodes = u32::from_be_bytes(read_next_n_bytes(&bytes, &mut pointer));
    for _ in 0..num_of_bytecodes {
        let current_bytecode_len =
            u32::from_be_bytes(read_next_n_bytes(&bytes, &mut pointer)) as usize;
        let bytecode = bytes[pointer..pointer + current_bytecode_len].to_vec();
        pointer += current_bytecode_len;
        l2_to_l1_pubdata.push(L2ToL1Pubdata::PublishedBytecode(bytecode))
    }

    // Parse compressed state diffs.
    let mut state_diffs = parse_compressed_state_diffs(&bytes, &mut pointer)?;
    l2_to_l1_pubdata.append(&mut state_diffs);

    Ok(l2_to_l1_pubdata)
}

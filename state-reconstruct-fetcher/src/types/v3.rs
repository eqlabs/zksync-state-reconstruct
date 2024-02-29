use ethers::{
    abi::{self},
    types::U256,
};
use serde::{Deserialize, Serialize};

use super::{
    common::{parse_compressed_state_diffs, read_next_n_bytes, ExtractedToken},
    CommitBlockFormat, CommitBlockInfo, L2ToL1Pubdata, ParseError,
};
use crate::constants::zksync::L2_TO_L1_LOG_SERIALIZE_SIZE;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PubdataSource {
    Calldata,
    Blob,
}

impl TryFrom<u8> for PubdataSource {
    type Error = ParseError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PubdataSource::Calldata),
            1 => Ok(PubdataSource::Blob),
            _ => Err(ParseError::InvalidPubdataSource(String::from(
                "InvalidPubdataSource",
            ))),
        }
    }
}

/// Data needed to commit new block
#[derive(Debug, Serialize, Deserialize)]
pub struct V3 {
    pub pubdata_source: PubdataSource,
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

impl CommitBlockFormat for V3 {
    fn to_enum_variant(self) -> CommitBlockInfo {
        CommitBlockInfo::V3(self)
    }
}

impl TryFrom<&abi::Token> for V3 {
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

        let mut pointer = 0;
        let pubdata_source = parse_pubdata_source(&total_l2_to_l1_pubdata, &mut pointer)?;
        let total_l2_to_l1_pubdata =
            parse_total_l2_to_l1_pubdata(&total_l2_to_l1_pubdata, &mut pointer, pubdata_source)?;
        let blk = V3 {
            pubdata_source,
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

// Read the source of the pubdata from a byte array.
fn parse_pubdata_source(bytes: &[u8], pointer: &mut usize) -> Result<PubdataSource, ParseError> {
    let pubdata_source = u8::from_be_bytes(read_next_n_bytes(bytes, pointer));
    pubdata_source.try_into()
}

fn parse_total_l2_to_l1_pubdata(
    bytes: &[u8],
    pointer: &mut usize,
    pubdata_source: PubdataSource,
) -> Result<Vec<L2ToL1Pubdata>, ParseError> {
    match pubdata_source {
        PubdataSource::Calldata => parse_pubdata_from_calldata(bytes, pointer),
        PubdataSource::Blob => todo!(),
    }
}

fn parse_pubdata_from_calldata(
    bytes: &[u8],
    pointer: &mut usize,
) -> Result<Vec<L2ToL1Pubdata>, ParseError> {
    let mut l2_to_l1_pubdata = Vec::new();

    // Skip over logs and messages.
    let num_of_l1_to_l2_logs = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
    *pointer += L2_TO_L1_LOG_SERIALIZE_SIZE * num_of_l1_to_l2_logs as usize;

    let num_of_messages = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
    for _ in 0..num_of_messages {
        let current_message_len = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
        *pointer += current_message_len as usize;
    }

    // Parse published bytecodes.
    let num_of_bytecodes = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
    for _ in 0..num_of_bytecodes {
        let current_bytecode_len = u32::from_be_bytes(read_next_n_bytes(bytes, pointer)) as usize;
        let bytecode = bytes[*pointer..*pointer + current_bytecode_len].to_vec();
        *pointer += current_bytecode_len;
        l2_to_l1_pubdata.push(L2ToL1Pubdata::PublishedBytecode(bytecode))
    }

    // Parse compressed state diffs.
    // NOTE: Is this correct? Ignoring the last 32 bytes?
    let mut state_diffs = parse_compressed_state_diffs(bytes, pointer, 32)?;
    l2_to_l1_pubdata.append(&mut state_diffs);

    Ok(l2_to_l1_pubdata)
}

use ethers::{abi, types::U256};
use serde::{Deserialize, Serialize};

use super::{CommitBlockFormat, CommitBlockInfo, ParseError};
use crate::constants::zksync::{
    L2_TO_L1_LOG_SERIALIZE_SIZE, LENGTH_BITS_OFFSET, OPERATION_BITMASK,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PackingType {
    Add(U256),
    Sub(U256),
    Transform(U256),
    NoCompression(U256),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum L2ToL1Pubdata {
    L2ToL1Log(Vec<u8>),
    L2ToL2Message(Vec<u8>),
    PublishedBytecode(Vec<u8>),
    CompressedStateDiff {
        is_repeated_write: bool,
        derived_key: U256,
        packing_type: PackingType,
    },
}

/// Data needed to commit new block
#[allow(dead_code)]
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
        let mut bytecode = Vec::new();
        bytecode.copy_from_slice(&bytes[pointer..pointer + current_bytecode_len]);
        pointer += current_bytecode_len;
        l2_to_l1_pubdata.push(L2ToL1Pubdata::PublishedBytecode(bytecode))
    }

    // Parse compressed state diffs.
    let mut state_diffs = parse_compressed_state_diffs(&bytes, &mut pointer)?;
    l2_to_l1_pubdata.append(&mut state_diffs);

    Ok(l2_to_l1_pubdata)
}

fn parse_compressed_state_diffs(
    bytes: &[u8],
    pointer: &mut usize,
) -> Result<Vec<L2ToL1Pubdata>, ParseError> {
    let mut state_diffs = Vec::new();
    // Parse the header.
    let version = u8::from_be_bytes(read_next_n_bytes(bytes, pointer));
    assert_eq!(version, 1);

    if *pointer >= bytes.len() {
        return Ok(state_diffs);
    }

    let mut buffer = [0; 4];
    buffer[1..].copy_from_slice(&bytes[*pointer..*pointer + 3]);
    *pointer += 3;
    let _total_compressed_len = u32::from_be_bytes(buffer);

    let enumeration_index = u8::from_be_bytes(read_next_n_bytes(bytes, pointer));

    // Parse initial writes.
    let num_of_initial_writes = u16::from_be_bytes(read_next_n_bytes(bytes, pointer));
    for _ in 0..num_of_initial_writes {
        let derived_key = U256::from_big_endian(&read_next_n_bytes::<32>(bytes, pointer));

        let packing_type = read_compressed_value(bytes, pointer)?;
        state_diffs.push(L2ToL1Pubdata::CompressedStateDiff {
            is_repeated_write: false,
            derived_key,
            packing_type,
        });
    }

    // Parse repeated writes.
    while *pointer < bytes.len() {
        let derived_key = match enumeration_index {
            4 => U256::from_big_endian(&read_next_n_bytes::<4>(bytes, pointer)),
            5 => U256::from_big_endian(&read_next_n_bytes::<5>(bytes, pointer)),
            _ => {
                return Err(ParseError::InvalidCompressedValue(String::from(
                    "RepeatedDerivedKey",
                )))
            }
        };

        let packing_type = read_compressed_value(bytes, pointer)?;
        state_diffs.push(L2ToL1Pubdata::CompressedStateDiff {
            is_repeated_write: true,
            derived_key,
            packing_type,
        });
    }

    Ok(state_diffs)
}

fn read_compressed_value(bytes: &[u8], pointer: &mut usize) -> Result<PackingType, ParseError> {
    let metadata = u8::from_be_bytes(read_next_n_bytes(bytes, pointer));
    let operation = metadata & OPERATION_BITMASK;
    let len = if operation == 0 {
        32
    } else {
        metadata >> LENGTH_BITS_OFFSET
    } as usize;

    // Read compressed value.
    let mut buffer = [0; 32];
    let start = buffer.len() - len;
    buffer[start..].copy_from_slice(&bytes[*pointer..*pointer + len]);
    *pointer += len;
    let compressed_value = U256::from_big_endian(&buffer);

    let packing_type = match operation {
        0 => PackingType::NoCompression(compressed_value),
        1 => PackingType::Add(compressed_value),
        2 => PackingType::Sub(compressed_value),
        3 => PackingType::Transform(compressed_value),
        _ => {
            return Err(ParseError::InvalidCompressedValue(String::from(
                "UnknownPackingType",
            )))
        }
    };

    Ok(packing_type)
}

fn read_next_n_bytes<const N: usize>(bytes: &[u8], pointer: &mut usize) -> [u8; N] {
    if *pointer >= bytes.len() {
        return [0; N];
    }
    let mut buffer = [0; N];
    buffer.copy_from_slice(&bytes[*pointer..*pointer + N]);
    *pointer += N;
    buffer
}

struct ExtractedToken {
    new_l2_block_number: U256,
    timestamp: U256,
    new_enumeration_index: U256,
    state_root: Vec<u8>,
    number_of_l1_txs: U256,
    priority_operations_hash: Vec<u8>,
    system_logs: Vec<u8>,
    total_l2_to_l1_pubdata: Vec<u8>,
}

impl TryFrom<&abi::Token> for ExtractedToken {
    type Error = ParseError;

    fn try_from(token: &abi::Token) -> Result<Self, Self::Error> {
        let abi::Token::Tuple(block_elems) = token else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "struct elements".to_string(),
            ));
        };

        let abi::Token::Uint(new_l2_block_number) = block_elems[0].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "blockNumber".to_string(),
            ));
        };

        let abi::Token::Uint(timestamp) = block_elems[1].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("timestamp".to_string()));
        };

        let abi::Token::Uint(new_enumeration_index) = block_elems[2].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "indexRepeatedStorageChanges".to_string(),
            ));
        };

        let abi::Token::FixedBytes(state_root) = block_elems[3].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "newStateRoot".to_string(),
            ));
        };

        let abi::Token::Uint(number_of_l1_txs) = block_elems[4].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "numberOfLayer1Txs".to_string(),
            ));
        };

        let abi::Token::FixedBytes(priority_operations_hash) = block_elems[5].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "priorityOperationsHash".to_string(),
            ));
        };

        let abi::Token::Bytes(system_logs) = block_elems[8].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("systemLogs".to_string()));
        };

        let abi::Token::Bytes(total_l2_to_l1_pubdata) = block_elems[9].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "totalL2ToL1Pubdata".to_string(),
            ));
        };

        Ok(Self {
            new_l2_block_number,
            timestamp,
            new_enumeration_index,
            state_root,
            number_of_l1_txs,
            priority_operations_hash,
            system_logs,
            total_l2_to_l1_pubdata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compressed_value_common() {
        let buf = [
            0, 1, 0, 0, 197, 168, 90, 55, 47, 68, 26, 198, 147, 33, 10, 24, 230, 131, 181, 48, 190,
            216, 117, 253, 202, 178, 247, 225, 1, 176, 87, 212, 51,
        ];
        let mut pointer = 0;
        let packing_type = read_compressed_value(&buf, &mut pointer).unwrap();
        let PackingType::NoCompression(_n) = packing_type else {
            panic!("packing_type = {:?}", packing_type);
        };
    }

    #[test]
    fn parse_compressed_value_add() {
        let buf = [9, 1];
        let mut pointer = 0;
        let packing_type = read_compressed_value(&buf, &mut pointer).unwrap();
        if let PackingType::Add(n) = packing_type {
            assert_eq!(n, U256::one());
        } else {
            panic!("packing_type = {:?}", packing_type);
        }
    }
}

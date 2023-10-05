use std::vec::Vec;

use ethers::{abi, types::U256};
use eyre::Result;
use indexmap::IndexMap;
use thiserror::Error;

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub enum ParseError {
    #[error("invalid Calldata: {0}")]
    InvalidCalldata(String),

    #[error("invalid StoredBlockInfo: {0}")]
    InvalidStoredBlockInfo(String),

    #[error("invalid CommitBlockInfo: {0}")]
    InvalidCommitBlockInfo(String),

    #[error("invalid compressed bytecode: {0}")]
    InvalidCompressedByteCode(String),
}

/// Data needed to commit new block
#[derive(Debug)]
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
    pub initial_storage_changes: IndexMap<[u8; 32], [u8; 32]>,
    /// Storage write access as a concatenation index-value.
    pub repeated_storage_changes: IndexMap<u64, [u8; 32]>,
    /// Concatenation of all L2 -> L1 logs in the block.
    pub l2_logs: Vec<u8>,
    /// (contract bytecodes) array of L2 bytecodes that were deployed.
    pub factory_deps: Vec<Vec<u8>>,
}

impl TryFrom<&abi::Token> for CommitBlockInfoV1 {
    type Error = ParseError;

    /// Try to parse Ethereum ABI token into CommitBlockInfoV1.
    ///
    /// * `token` - ABI token of `CommitBlockInfo` type on Ethereum.
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

        /* TODO(tuommaki): Fix the check below.
        if new_l2_block_number <= latest_l2_block_number {
            println!("skipping before we even get started");
            continue;
        }
        */

        let abi::Token::Uint(timestamp) = block_elems[1].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("timestamp".to_string()));
        };

        let abi::Token::Uint(new_enumeration_index) = block_elems[2].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "indexRepeatedStorageChanges".to_string(),
            ));
        };
        let new_enumeration_index = new_enumeration_index.0[0];

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

        let abi::Token::FixedBytes(l2_logs_tree_root) = block_elems[5].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "l2LogsTreeRoot".to_string(),
            ));
        };

        let abi::Token::FixedBytes(priority_operations_hash) = block_elems[6].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "priorityOperationsHash".to_string(),
            ));
        };

        let abi::Token::Bytes(initial_changes_calldata) = block_elems[7].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "initialStorageChanges".to_string(),
            ));
        };

        if initial_changes_calldata.len() % 64 != 4 {
            return Err(ParseError::InvalidCommitBlockInfo(
                "initialStorageChanges".to_string(),
            ));
        }
        let abi::Token::Bytes(repeated_changes_calldata) = block_elems[8].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "repeatedStorageChanges".to_string(),
            ));
        };

        let abi::Token::Bytes(l2_logs) = block_elems[9].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("l2Logs".to_string()));
        };

        // TODO(tuommaki): Are these useful at all?
        /*
        let abi::Token::Bytes(_l2_arbitrary_length_msgs) = block_elems[10].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "l2ArbitraryLengthMessages".to_string(),
            )
            );
        };
        */

        // TODO(tuommaki): Parse factory deps
        let abi::Token::Array(factory_deps) = block_elems[11].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "factoryDeps".to_string(),
            ));
        };

        let mut smartcontracts = vec![];
        for bytecode in factory_deps.into_iter() {
            let abi::Token::Bytes(bytecode) = bytecode else {
                return Err(ParseError::InvalidCommitBlockInfo(
                    "factoryDeps".to_string(),
                ));
            };

            match decompress_bytecode(bytecode) {
                Ok(bytecode) => smartcontracts.push(bytecode),
                Err(e) => println!("failed to decompress bytecode: {}", e),
            };
        }

        assert_eq!(repeated_changes_calldata.len() % 40, 4);

        println!(
            "Have {} new keys",
            (initial_changes_calldata.len() - 4) / 64
        );
        println!(
            "Have {} repeated keys",
            (repeated_changes_calldata.len() - 4) / 40
        );

        let mut blk = CommitBlockInfoV1 {
            block_number: new_l2_block_number.as_u64(),
            timestamp: timestamp.as_u64(),
            index_repeated_storage_changes: new_enumeration_index,
            new_state_root: state_root,
            number_of_l1_txs,
            l2_logs_tree_root,
            priority_operations_hash,
            initial_storage_changes: IndexMap::default(),
            repeated_storage_changes: IndexMap::default(),
            l2_logs: l2_logs.to_vec(),
            factory_deps: smartcontracts,
        };

        for initial_calldata in initial_changes_calldata[4..].chunks(64) {
            let mut t = initial_calldata.array_chunks::<32>();
            let key = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("initialStorageChanges".to_string())
            })?;
            let value = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("initialStorageChanges".to_string())
            })?;

            if t.next().is_some() {
                return Err(ParseError::InvalidCommitBlockInfo(
                    "initialStorageChanges".to_string(),
                ));
            }

            let _ = blk.initial_storage_changes.insert(key, value);
        }

        for repeated_calldata in repeated_changes_calldata[4..].chunks(40) {
            let index = u64::from_be_bytes([
                repeated_calldata[0],
                repeated_calldata[1],
                repeated_calldata[2],
                repeated_calldata[3],
                repeated_calldata[4],
                repeated_calldata[5],
                repeated_calldata[6],
                repeated_calldata[7],
            ]);
            let mut t = repeated_calldata[8..].array_chunks::<32>();
            let value = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("repeatedStorageChanges".to_string())
            })?;

            if t.next().is_some() {
                return Err(ParseError::InvalidCommitBlockInfo(
                    "repeatedStorageChanges".to_string(),
                ));
            }

            blk.repeated_storage_changes.insert(index, value);
        }

        Ok(blk)
    }
}

pub fn decompress_bytecode(data: Vec<u8>) -> Result<Vec<u8>> {
    let dict_len = u16::from_be_bytes([data[0], data[1]]);
    let end = 2 + dict_len as usize * 8;
    let dict = data[2..end].to_vec();
    let encoded_data = data[end..].to_vec();

    let dict: Vec<&[u8]> = dict.chunks(8).collect();

    // Verify that dictionary size is below maximum.
    if dict.len() > (1 << 16)
    /* 2^16 */
    {
        return Err(ParseError::InvalidCompressedByteCode(format!(
            "too many elements in dictionary: {}",
            dict.len()
        ))
        .into());
    }

    let mut bytecode = vec![];
    for idx in encoded_data.chunks(2) {
        let idx = u16::from_be_bytes([idx[0], idx[1]]) as usize;

        if dict.len() <= idx {
            return Err(ParseError::InvalidCompressedByteCode(format!(
                "encoded data index ({}) exceeds dictionary size ({})",
                idx,
                dict.len()
            ))
            .into());
        }

        bytecode.append(&mut dict[idx].to_vec());
    }

    Ok(bytecode)
}

#[allow(dead_code)]
pub enum L2ToL1Pubdata {
    L2ToL1Log,
    L2ToL2Message,
    PublishedBytecode,
    CompressedStateDiff,
}

/// Data needed to commit new block
#[allow(dead_code)]
pub struct CommitBlockInfoV2 {
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

impl CommitBlockInfoV1 {
    #[allow(dead_code)]
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

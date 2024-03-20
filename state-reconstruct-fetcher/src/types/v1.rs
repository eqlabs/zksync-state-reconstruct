use ethers::{abi, types::U256};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json_any_key::any_key_map;

use super::{CommitBlockFormat, CommitBlockInfo, ParseError};

/// Data needed to commit new block
#[derive(Debug, Serialize, Deserialize)]
pub struct V1 {
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
    #[serde(with = "any_key_map")]
    pub initial_storage_changes: IndexMap<[u8; 32], [u8; 32]>,
    /// Storage write access as a concatenation index-value.
    #[serde(with = "any_key_map")]
    pub repeated_storage_changes: IndexMap<u64, [u8; 32]>,
    /// Concatenation of all L2 -> L1 logs in the block.
    pub l2_logs: Vec<u8>,
    /// (contract bytecodes) array of L2 bytecodes that were deployed.
    pub factory_deps: Vec<Vec<u8>>,
}

impl CommitBlockFormat for V1 {
    fn to_enum_variant(self) -> CommitBlockInfo {
        CommitBlockInfo::V1(self)
    }
}

impl TryFrom<&abi::Token> for V1 {
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
            l2_logs_tree_root,
            priority_operations_hash,
            initial_changes_calldata,
            repeated_changes_calldata,
            l2_logs,
            factory_deps,
        } = token.try_into()?;
        let new_enumeration_index = new_enumeration_index.0[0];

        let mut smartcontracts = vec![];
        for bytecode in &factory_deps {
            let abi::Token::Bytes(bytecode) = bytecode else {
                return Err(ParseError::InvalidCommitBlockInfo(
                    "factoryDeps".to_string(),
                ));
            };

            smartcontracts.push(bytecode.clone());
        }

        assert_eq!(repeated_changes_calldata.len() % 40, 4);

        tracing::trace!(
            "Have {} new keys",
            (initial_changes_calldata.len() - 4) / 64
        );
        tracing::trace!(
            "Have {} repeated keys",
            (repeated_changes_calldata.len() - 4) / 40
        );

        let mut blk = V1 {
            block_number: new_l2_block_number.as_u64(),
            timestamp: timestamp.as_u64(),
            index_repeated_storage_changes: new_enumeration_index,
            new_state_root: state_root,
            number_of_l1_txs,
            l2_logs_tree_root,
            priority_operations_hash,
            initial_storage_changes: IndexMap::default(),
            repeated_storage_changes: IndexMap::default(),
            l2_logs: l2_logs.clone(),
            factory_deps: smartcontracts,
        };

        for initial_calldata in initial_changes_calldata[4..].chunks(64) {
            let mut t = initial_calldata.array_chunks::<32>();
            let key = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("initialStorageChangesKey".to_string())
            })?;
            let value = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("initialStorageChangesValue".to_string())
            })?;

            if t.next().is_some() {
                return Err(ParseError::InvalidCommitBlockInfo(
                    "initialStorageChangesMulti".to_string(),
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

struct ExtractedToken {
    new_l2_block_number: U256,
    timestamp: U256,
    new_enumeration_index: U256,
    state_root: Vec<u8>,
    number_of_l1_txs: U256,
    l2_logs_tree_root: Vec<u8>,
    priority_operations_hash: Vec<u8>,
    initial_changes_calldata: Vec<u8>,
    repeated_changes_calldata: Vec<u8>,
    l2_logs: Vec<u8>,
    factory_deps: Vec<abi::Token>,
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
                "initialStorageChangesParam".to_string(),
            ));
        };

        if initial_changes_calldata.len() % 64 != 4 {
            return Err(ParseError::InvalidCommitBlockInfo(
                "initialStorageChangesLength".to_string(),
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

        Ok(Self {
            new_l2_block_number,
            timestamp,
            new_enumeration_index,
            state_root,
            number_of_l1_txs,
            l2_logs_tree_root,
            priority_operations_hash,
            initial_changes_calldata,
            repeated_changes_calldata,
            l2_logs,
            factory_deps,
        })
    }
}

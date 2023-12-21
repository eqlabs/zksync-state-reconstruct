use ethers::abi;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use self::{v1::V1, v2::V2};

pub mod v1;
pub mod v2;

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub enum ParseError {
    #[error("invalid Calldata: {0}")]
    InvalidCalldata(String),

    #[error("invalid StoredBlockInfo: {0}")]
    InvalidStoredBlockInfo(String),

    #[error("invalid CommitBlockInfo: {0}")]
    InvalidCommitBlockInfo(String),

    #[allow(dead_code)]
    #[error("invalid compressed bytecode: {0}")]
    InvalidCompressedByteCode(String),

    #[error("invalid compressed value: {0}")]
    InvalidCompressedValue(String),
}

pub trait CommitBlockFormat {
    fn to_enum_variant(self) -> CommitBlockInfo;
}

#[derive(Debug)]
pub enum CommitBlockInfo {
    V1(V1),
    V2(V2),
}

/// Block with all required fields extracted from a [`CommitBlockInfo`].
#[derive(Debug, Serialize, Deserialize)]
pub struct CommitBlock {
    /// L1 block number.
    pub l1_block_number: Option<u64>,
    /// L2 block number.
    pub l2_block_number: u64,
    /// The state root of the full state tree.
    pub new_state_root: Vec<u8>,
    /// Storage write access as a concatenation key-value.
    pub initial_storage_changes: IndexMap<[u8; 32], [u8; 32]>,
    /// Storage write access as a concatenation index-value.
    pub repeated_storage_changes: IndexMap<u64, [u8; 32]>,
    /// (contract bytecodes) array of L2 bytecodes that were deployed.
    pub factory_deps: Vec<Vec<u8>>,
}

impl CommitBlock {
    pub fn try_from_token<'a, F>(value: &'a abi::Token) -> Result<Self, ParseError>
    where
        F: CommitBlockFormat + TryFrom<&'a abi::Token, Error = ParseError>,
    {
        let commit_block_info = F::try_from(value).unwrap().to_enum_variant();
        Ok(Self::from_commit_block(commit_block_info))
    }

    pub fn from_commit_block(block_type: CommitBlockInfo) -> Self {
        match block_type {
            CommitBlockInfo::V1(block) => CommitBlock {
                l1_block_number: None,
                l2_block_number: block.block_number,
                new_state_root: block.new_state_root,
                initial_storage_changes: block.initial_storage_changes,
                repeated_storage_changes: block.repeated_storage_changes,
                factory_deps: block.factory_deps,
            },
            CommitBlockInfo::V2(_block) => todo!(),
        }
    }
}

use blobscan_client::BlobError;
use ethers::{abi, types::U256};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json_any_key::any_key_map;
use thiserror::Error;

use self::{v1::V1, v2::V2, v3::V3};
use crate::blob_http_client::BlobHttpClient;

// NOTE: We should probably make these more human-readable.
pub mod common;
pub mod v1;
pub mod v2;
pub mod v3;

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

    #[error("invalid pubdata source: {0}")]
    InvalidPubdataSource(String),

    #[error("blob storage error: {0}")]
    BlobStorageError(String),

    #[error("blob format error: {0}")]
    BlobFormatError(String, String),
}

impl From<BlobError> for ParseError {
    fn from(opt: BlobError) -> Self {
        match opt {
            BlobError::StorageError(code) => ParseError::BlobStorageError(code),
            BlobError::FormatError(data, msg) => ParseError::BlobFormatError(data, msg),
        }
    }
}

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
    #[serde(skip)]
    pub l1_block_number: Option<u64>,
    /// L2 block number.
    pub l2_block_number: u64,
    /// Next unused key serial number.
    pub index_repeated_storage_changes: u64,
    /// The state root of the full state tree.
    pub new_state_root: Vec<u8>,
    /// Storage write access as a concatenation key-value.
    #[serde(with = "any_key_map")]
    pub initial_storage_changes: IndexMap<[u8; 32], PackingType>,
    /// Storage write access as a concatenation index-value.
    #[serde(with = "any_key_map")]
    pub repeated_storage_changes: IndexMap<u64, PackingType>,
    /// (contract bytecodes) array of L2 bytecodes that were deployed.
    pub factory_deps: Vec<Vec<u8>>,
}

impl CommitBlock {
    pub fn try_from_token<'a, F>(value: &'a abi::Token) -> Result<Self, ParseError>
    where
        F: CommitBlockFormat + TryFrom<&'a abi::Token, Error = ParseError>,
    {
        let commit_block_info = F::try_from(value)?;
        Ok(Self::from_commit_block(commit_block_info.to_enum_variant()))
    }

    pub async fn try_from_token_resolve<'a>(
        value: &'a abi::Token,
        client: &BlobHttpClient,
    ) -> Result<Self, ParseError> {
        let commit_block_info = V3::try_from(value)?;
        Self::from_commit_block_resolve(commit_block_info, client).await
    }

    pub fn from_commit_block(block_type: CommitBlockInfo) -> Self {
        match block_type {
            CommitBlockInfo::V1(block) => CommitBlock {
                l1_block_number: None,
                l2_block_number: block.block_number,
                index_repeated_storage_changes: block.index_repeated_storage_changes,
                new_state_root: block.new_state_root,
                initial_storage_changes: block
                    .initial_storage_changes
                    .into_iter()
                    .map(|(k, v)| (k, PackingType::NoCompression(v.into())))
                    .collect(),
                repeated_storage_changes: block
                    .repeated_storage_changes
                    .into_iter()
                    .map(|(k, v)| (k, PackingType::NoCompression(v.into())))
                    .collect(),
                factory_deps: block.factory_deps,
            },
            CommitBlockInfo::V2(block) => {
                let mut initial_storage_changes = IndexMap::new();
                let mut repeated_storage_changes = IndexMap::new();
                let mut factory_deps = Vec::new();
                for log in block.total_l2_to_l1_pubdata {
                    match log {
                        L2ToL1Pubdata::L2ToL1Log(_) | L2ToL1Pubdata::L2ToL2Message(_) => (),
                        L2ToL1Pubdata::PublishedBytecode(bytecode) => factory_deps.push(bytecode),
                        L2ToL1Pubdata::CompressedStateDiff {
                            is_repeated_write,
                            derived_key,
                            packing_type,
                        } => {
                            let mut key = [0u8; 32];
                            derived_key.to_big_endian(&mut key);

                            if is_repeated_write {
                                repeated_storage_changes.insert(derived_key.as_u64(), packing_type);
                            } else {
                                initial_storage_changes.insert(key, packing_type);
                            };
                        }
                    }
                }

                CommitBlock {
                    l1_block_number: None,
                    l2_block_number: block.block_number,
                    index_repeated_storage_changes: block.index_repeated_storage_changes,
                    new_state_root: block.new_state_root,
                    initial_storage_changes,
                    repeated_storage_changes,
                    factory_deps,
                }
            }
        }
    }

    pub async fn from_commit_block_resolve(
        block: V3,
        client: &BlobHttpClient,
    ) -> Result<Self, ParseError> {
        let total_l2_to_l1_pubdata = block.parse_pubdata(client).await?;
        let mut initial_storage_changes = IndexMap::new();
        let mut repeated_storage_changes = IndexMap::new();
        let mut factory_deps = Vec::new();
        for log in total_l2_to_l1_pubdata {
            match log {
                L2ToL1Pubdata::L2ToL1Log(_) | L2ToL1Pubdata::L2ToL2Message(_) => (),
                L2ToL1Pubdata::PublishedBytecode(bytecode) => factory_deps.push(bytecode),
                L2ToL1Pubdata::CompressedStateDiff {
                    is_repeated_write,
                    derived_key,
                    packing_type,
                } => {
                    let mut key = [0u8; 32];
                    derived_key.to_big_endian(&mut key);

                    if is_repeated_write {
                        repeated_storage_changes.insert(derived_key.as_u64(), packing_type);
                    } else {
                        initial_storage_changes.insert(key, packing_type);
                    };
                }
            }
        }

        Ok(CommitBlock {
            l1_block_number: None,
            l2_block_number: block.block_number,
            index_repeated_storage_changes: block.index_repeated_storage_changes,
            new_state_root: block.new_state_root,
            initial_storage_changes,
            repeated_storage_changes,
            factory_deps,
        })
    }
}

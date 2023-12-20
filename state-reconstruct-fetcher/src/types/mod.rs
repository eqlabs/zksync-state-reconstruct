use eyre::Result;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub async fn from_commit_block(block_type: CommitBlockInfo) -> Self {
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

#[derive(Debug)]
pub enum CommitBlockInfo {
    V1(v1::CommitBlockInfo),
    V2(v2::CommitBlockInfo),
}

// TODO: Do we need this?
#[allow(dead_code)]
fn decompress_bytecode(data: &[u8]) -> Result<Vec<u8>> {
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

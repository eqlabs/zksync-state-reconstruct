#![feature(array_chunks)]
#![feature(iter_next_chunk)]
use thiserror::Error;

pub mod blob_http_client;
pub mod constants;
pub mod database;
pub mod l1_fetcher;
pub mod metrics;
pub mod parser;
pub mod types;

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

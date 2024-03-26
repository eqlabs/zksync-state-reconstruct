use thiserror::Error;

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub enum BlobError {
    #[error("blob storage error: {0}")]
    StorageError(String),

    #[error("blob format error")]
    FormatError(String, String),
}

pub trait BlobSupport {
    fn format_url(&self, kzg_commitment: &[u8]) -> String;

    fn get_blob_data(&self, json_str: &str) -> Result<String, BlobError>;
}

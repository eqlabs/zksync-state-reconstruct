use std::fmt;

use thiserror::Error;

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub struct BlobResponseFormatError(pub String, pub String);

impl fmt::Display for BlobResponseFormatError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.0, self.1)
    }
}

pub trait BlobSupport {
    fn format_url(&self, kzg_commitment: &[u8]) -> String;

    fn get_blob_data(&self, json_str: &str) -> Result<String, BlobResponseFormatError>;
}

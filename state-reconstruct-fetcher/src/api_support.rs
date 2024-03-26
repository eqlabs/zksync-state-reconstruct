use blobscan_client::{BlobError, BlobSupport};
use serde::Deserialize;

#[derive(Deserialize)]
struct JsonResponse {
    data: String,
}

pub struct ApiSupport {
    url_base: String,
}

impl ApiSupport {
    pub fn new(blob_url: String) -> Self {
        Self { url_base: blob_url }
    }
}

impl BlobSupport for ApiSupport {
    fn format_url(&self, kzg_commitment: &[u8]) -> String {
        format!("{}0x{}", self.url_base, hex::encode(kzg_commitment))
    }

    fn get_blob_data(&self, json_str: &str) -> Result<String, BlobError> {
        match serde_json::from_str::<JsonResponse>(json_str) {
            Ok(data) => Ok(data.data),
            Err(e) => Err(BlobError::FormatError(json_str.to_string(), e.to_string())),
        }
    }
}

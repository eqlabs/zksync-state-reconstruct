use std::fs;

use blobscan_client::{BlobResponseFormatError, BlobSupport};
use tokio::time::{sleep, Duration};

use crate::types::ParseError;

/// `MAX_RETRIES` is the maximum number of retries on failed blob retrieval.
const MAX_RETRIES: u8 = 5;
/// The interval in seconds to wait before retrying to fetch a blob.
const FAILED_FETCH_RETRY_INTERVAL_S: u64 = 10;

pub struct BlobHttpClient {
    client: reqwest::Client,
    support: Box<dyn BlobSupport + Send + Sync>,
}

impl BlobHttpClient {
    pub fn new(support: Box<dyn BlobSupport + Send + Sync>) -> eyre::Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Accept",
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(Self { client, support })
    }

    pub async fn get_blob(&self, kzg_commitment: &[u8]) -> Result<Vec<u8>, ParseError> {
        let url = self.support.format_url(kzg_commitment);
        for attempt in 1..=MAX_RETRIES {
            match self.client.get(&url).send().await {
                Ok(response) => match response.text().await {
                    Ok(text) => match self.support.get_blob_data(&text) {
                        Ok(data) => {
                            let plain = if let Some(p) = data.strip_prefix("0x") {
                                p
                            } else {
                                &data
                            };
                            return hex::decode(plain).map_err(|e| {
                                BlobResponseFormatError(plain.to_string(), e.to_string()).into()
                            });
                        }
                        Err(e) => {
                            tracing::error!("failed parsing response of {url}");
                            return self.get_blob_backup(kzg_commitment, e);
                        }
                    },
                    Err(e) => {
                        tracing::error!("attempt {}: {} failed: {:?}", attempt, url, e);
                        sleep(Duration::from_secs(FAILED_FETCH_RETRY_INTERVAL_S)).await;
                    }
                },
                Err(e) => {
                    tracing::error!("attempt {}: GET {} failed: {:?}", attempt, url, e);
                    sleep(Duration::from_secs(FAILED_FETCH_RETRY_INTERVAL_S)).await;
                }
            }
        }
        Err(ParseError::BlobStorageError(url))
    }

    fn get_blob_backup(
        &self,
        kzg_commitment: &[u8],
        orig_err: BlobResponseFormatError,
    ) -> Result<Vec<u8>, ParseError> {
        let backup_file = format!("known/0x{}", hex::encode(kzg_commitment));
        match fs::read_to_string(backup_file) {
            Ok(data) => {
                let data = data.trim();
                let plain = if let Some(p) = data.strip_prefix("0x") {
                    p
                } else {
                    data
                };
                hex::decode(plain)
                    .map_err(|e| BlobResponseFormatError(plain.to_string(), e.to_string()).into())
            }
            Err(_) => Err(orig_err.into()),
        }
    }
}

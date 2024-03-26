use blobscan_client::{BlobError, BlobSupport};
use tokio::time::{sleep, Duration};

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

    pub async fn get_blob(&self, kzg_commitment: &[u8]) -> Result<Vec<u8>, BlobError> {
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
                                BlobError::FormatError(plain.to_string(), e.to_string())
                            });
                        }
                        Err(e) => {
                            tracing::error!("failed parsing response of {url}");
                            return Err(e);
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
        Err(BlobError::StorageError(url))
    }
}

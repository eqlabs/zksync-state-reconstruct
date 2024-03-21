use serde::Deserialize;
use tokio::time::{sleep, Duration};

use crate::ParseError;

/// `MAX_RETRIES` is the maximum number of retries on failed blob retrieval.
const MAX_RETRIES: u8 = 5;
/// The interval in seconds to wait before retrying to fetch a blob.
const FAILED_FETCH_RETRY_INTERVAL_S: u64 = 10;

#[derive(Deserialize)]
struct JsonResponse {
    data: String,
}

pub struct BlobHttpClient {
    client: reqwest::Client,
    url_base: String,
}

impl BlobHttpClient {
    pub fn new(blob_url: String) -> eyre::Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Accept",
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(Self {
            client,
            url_base: blob_url,
        })
    }

    pub async fn get_blob(&self, kzg_commitment: &[u8]) -> Result<Vec<u8>, ParseError> {
        let url = self.format_url(kzg_commitment);
        for attempt in 1..=MAX_RETRIES {
            match self.retrieve_url(&url).await {
                Ok(response) => match response.text().await {
                    Ok(text) => match get_blob_data(&text) {
                        Ok(data) => {
                            let plain = if let Some(p) = data.strip_prefix("0x") {
                                p
                            } else {
                                &data
                            };
                            return hex::decode(plain).map_err(|e| {
                                ParseError::BlobFormatError(plain.to_string(), e.to_string())
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
        Err(ParseError::BlobStorageError(url))
    }

    fn format_url(&self, kzg_commitment: &[u8]) -> String {
        format!("{}0x{}", self.url_base, hex::encode(kzg_commitment))
    }

    async fn retrieve_url(&self, url: &str) -> eyre::Result<reqwest::Response> {
        let result = self.client.get(url).send().await?;
        Ok(result)
    }
}

fn get_blob_data(json_str: &str) -> Result<String, ParseError> {
    match serde_json::from_str::<JsonResponse>(json_str) {
        Ok(data) => Ok(data.data),
        Err(e) => Err(ParseError::BlobFormatError(
            json_str.to_string(),
            e.to_string(),
        )),
    }
}

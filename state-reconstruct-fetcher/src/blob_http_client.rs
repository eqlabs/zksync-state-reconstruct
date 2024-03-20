use eyre::Result;

pub struct BlobHttpClient {
    client: reqwest::Client,
    url_base: String,
}

impl BlobHttpClient {
    pub fn new(blob_url: String) -> Result<Self> {
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

    pub fn format_url(&self, kzg_commitment: &[u8]) -> String {
        format!("{}0x{}", self.url_base, hex::encode(kzg_commitment))
    }

    pub async fn retrieve_url(&self, url: &str) -> Result<reqwest::Response> {
        let result = self.client.get(url).send().await?;
        Ok(result)
    }
}

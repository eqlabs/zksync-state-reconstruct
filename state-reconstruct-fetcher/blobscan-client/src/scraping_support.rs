use serde::Deserialize;
use serde_json::json;

use crate::blob_support::{BlobError, BlobSupport};

#[derive(Deserialize)]
struct JsonResponse {
    result: JsonResponseResult,
}

#[derive(Deserialize)]
struct JsonResponseResult {
    data: JsonResponseData,
}

#[derive(Deserialize)]
struct JsonResponseData {
    json: JsonResponseJson,
}

#[derive(Deserialize)]
struct JsonResponseJson {
    data: String,
}

#[derive(Default)]
pub struct ScrapingSupport {}

impl BlobSupport for ScrapingSupport {
    fn format_url(&self, kzg_commitment: &[u8]) -> String {
        let id = format!("0x{}", hex::encode(kzg_commitment));
        let json_source = json!({
            "json": {
                "id": id
            }
        });
        let json_string = json_source.to_string();
        let url = reqwest::Url::parse_with_params(
            "https://blobscan.com/api/trpc/blob.getByBlobIdFull",
            &[("input", &json_string)],
        )
        .unwrap();
        url.to_string()
    }

    fn get_blob_data(&self, json_str: &str) -> Result<String, BlobError> {
        match serde_json::from_str::<JsonResponse>(json_str) {
            Ok(rsp) => Ok(rsp.result.data.json.data),
            Err(e) => Err(BlobError::FormatError(json_str.to_string(), e.to_string())),
        }
    }
}

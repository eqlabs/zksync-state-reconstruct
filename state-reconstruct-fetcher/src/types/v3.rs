use ethers::{
    abi::{self},
    types::U256,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::{sleep, Duration};
use zkevm_circuits::eip_4844::ethereum_4844_data_into_zksync_pubdata;

use super::{
    common::{parse_compressed_state_diffs, read_next_n_bytes, ExtractedToken},
    L2ToL1Pubdata, ParseError,
};
use crate::constants::zksync::{L2_TO_L1_LOG_SERIALIZE_SIZE, PUBDATA_COMMITMENT_SIZE};

/// `MAX_RETRIES` is the maximum number of retries on failed blob retrieval.
const MAX_RETRIES: u8 = 5;
/// The interval in seconds to wait before retrying to fetch a blob.
const FAILED_FETCH_RETRY_INTERVAL_S: u64 = 10;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PubdataSource {
    Calldata,
    Blob,
}

impl TryFrom<u8> for PubdataSource {
    type Error = ParseError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PubdataSource::Calldata),
            1 => Ok(PubdataSource::Blob),
            _ => Err(ParseError::InvalidPubdataSource(String::from(
                "InvalidPubdataSource",
            ))),
        }
    }
}

/// Data needed to commit new block
#[derive(Debug, Serialize, Deserialize)]
pub struct V3 {
    pub pubdata_source: PubdataSource,
    /// L2 block number.
    pub block_number: u64,
    /// Unix timestamp denoting the start of the block execution.
    pub timestamp: u64,
    /// The serial number of the shortcut index that's used as a unique identifier for storage keys that were used twice or more.
    pub index_repeated_storage_changes: u64,
    /// The state root of the full state tree.
    pub new_state_root: Vec<u8>,
    /// Number of priority operations to be processed.
    pub number_of_l1_txs: U256,
    /// Hash of all priority operations from this block.
    pub priority_operations_hash: Vec<u8>,
    /// Concatenation of all L2 -> L1 system logs in the block.
    pub system_logs: Vec<u8>,
    /// Unparsed blob commitments; must be either parsed, or parsed and resolved using some blob storage server (depending on `pubdata_source`).
    pub pubdata_commitments: Vec<u8>,
}

impl TryFrom<&abi::Token> for V3 {
    type Error = ParseError;

    /// Try to parse Ethereum ABI token.
    fn try_from(token: &abi::Token) -> Result<Self, Self::Error> {
        let ExtractedToken {
            new_l2_block_number,
            timestamp,
            new_enumeration_index,
            state_root,
            number_of_l1_txs,
            priority_operations_hash,
            system_logs,
            total_l2_to_l1_pubdata,
        } = token.try_into()?;
        let new_enumeration_index = new_enumeration_index.as_u64();

        let mut pointer = 0;
        let pubdata_source = parse_pubdata_source(&total_l2_to_l1_pubdata, &mut pointer)?;
        let pubdata_commitments =
            total_l2_to_l1_pubdata[pointer..total_l2_to_l1_pubdata.len()].to_vec();
        let blk = V3 {
            pubdata_source,
            block_number: new_l2_block_number.as_u64(),
            timestamp: timestamp.as_u64(),
            index_repeated_storage_changes: new_enumeration_index,
            new_state_root: state_root,
            number_of_l1_txs,
            priority_operations_hash,
            system_logs,
            pubdata_commitments,
        };

        Ok(blk)
    }
}

impl V3 {
    pub async fn parse_pubdata(
        &self,
        client: &reqwest::Client,
        blobs_url: &str,
    ) -> Result<Vec<L2ToL1Pubdata>, ParseError> {
        let mut pointer = 0;
        let bytes = &self.pubdata_commitments[..];
        match self.pubdata_source {
            PubdataSource::Calldata => parse_pubdata_from_calldata(bytes, &mut pointer, true),
            PubdataSource::Blob => {
                parse_pubdata_from_blobs(bytes, &mut pointer, client, blobs_url).await
            }
        }
    }
}

// Read the source of the pubdata from a byte array.
fn parse_pubdata_source(bytes: &[u8], pointer: &mut usize) -> Result<PubdataSource, ParseError> {
    let pubdata_source = u8::from_be_bytes(read_next_n_bytes(bytes, pointer));
    pubdata_source.try_into()
}

fn parse_pubdata_from_calldata(
    bytes: &[u8],
    pointer: &mut usize,
    shorten: bool,
) -> Result<Vec<L2ToL1Pubdata>, ParseError> {
    let mut l2_to_l1_pubdata = Vec::new();

    // Skip over logs and messages.
    let num_of_l1_to_l2_logs = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
    *pointer += L2_TO_L1_LOG_SERIALIZE_SIZE * num_of_l1_to_l2_logs as usize;

    let num_of_messages = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
    for _ in 0..num_of_messages {
        let current_message_len = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
        *pointer += current_message_len as usize;
    }

    // Parse published bytecodes.
    let num_of_bytecodes = u32::from_be_bytes(read_next_n_bytes(bytes, pointer));
    for _ in 0..num_of_bytecodes {
        let current_bytecode_len = u32::from_be_bytes(read_next_n_bytes(bytes, pointer)) as usize;
        let bytecode = bytes[*pointer..*pointer + current_bytecode_len].to_vec();
        *pointer += current_bytecode_len;
        l2_to_l1_pubdata.push(L2ToL1Pubdata::PublishedBytecode(bytecode))
    }

    // Parse compressed state diffs.
    // NOTE: Is this correct? Ignoring the last 32 bytes?
    let diff_bytes = if shorten {
        let end_point = bytes.len() - 32;
        &bytes[..end_point]
    } else {
        bytes
    };
    let mut state_diffs = parse_compressed_state_diffs(diff_bytes, pointer)?;
    l2_to_l1_pubdata.append(&mut state_diffs);

    Ok(l2_to_l1_pubdata)
}

async fn parse_pubdata_from_blobs(
    bytes: &[u8],
    pointer: &mut usize,
    client: &reqwest::Client,
    blobs_url: &str,
) -> Result<Vec<L2ToL1Pubdata>, ParseError> {
    let mut l = bytes.len() - *pointer;
    let mut blobs = Vec::new();
    while *pointer < l {
        let pubdata_commitment = &bytes[*pointer..*pointer + PUBDATA_COMMITMENT_SIZE];
        let blob = get_blob(&pubdata_commitment[48..96], client, blobs_url).await?;
        let mut blob_bytes = ethereum_4844_data_into_zksync_pubdata(&blob);
        blobs.append(&mut blob_bytes);
        *pointer += PUBDATA_COMMITMENT_SIZE;
    }

    l = blobs.len();
    while l > 0 && blobs[l - 1] == 0u8 {
        l -= 1;
    }

    let blobs_view = &blobs[..l];
    let mut pointer = 0;
    parse_pubdata_from_calldata(blobs_view, &mut pointer, false)
}

async fn get_blob(
    kzg_commitment: &[u8],
    client: &reqwest::Client,
    blobs_url: &str,
) -> Result<Vec<u8>, ParseError> {
    let url = format!("{}0x{}", blobs_url, hex::encode(kzg_commitment));
    for attempt in 1..=MAX_RETRIES {
        match client.get(url.clone()).send().await {
            Ok(response) => match response.text().await {
                Ok(text) => match get_blob_data(&text) {
                    Ok(data) => {
                        let plain = if let Some(p) = data.strip_prefix("0x") {
                            p
                        } else {
                            &data
                        };
                        match hex::decode(plain) {
                            Ok(bytes) => return Ok(bytes),
                            Err(e) => {
                                tracing::warn!("Cannot parse {}: {:?}", plain, e);
                                return Err(ParseError::BlobFormatError("not hex".to_string()));
                            }
                        }
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

fn get_blob_data(json_str: &str) -> Result<String, ParseError> {
    if let Ok(v) = serde_json::from_str(json_str) {
        if let Value::Object(m) = v {
            if let Some(d) = m.get("data") {
                if let Value::String(s) = d {
                    Ok(s.clone())
                } else {
                    tracing::warn!("Cannot parse {json_str} - data is not string.");
                    Err(ParseError::BlobFormatError(
                        "data is not string".to_string(),
                    ))
                }
            } else {
                tracing::warn!("Cannot parse {json_str} - no data in response.");
                Err(ParseError::BlobFormatError(
                    "no data in response".to_string(),
                ))
            }
        } else {
            tracing::warn!("Cannot parse {json_str} - data is not object.");
            Err(ParseError::BlobFormatError(
                "data is not object".to_string(),
            ))
        }
    } else {
        tracing::warn!("Cannot parse {json_str} - not JSON.");
        Err(ParseError::BlobFormatError("not JSON".to_string()))
    }
}

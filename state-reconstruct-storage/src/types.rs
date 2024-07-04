use std::{
    io::{Read, Write},
    path::Path,
};

use bytes::BytesMut;
use chrono::{offset::Utc, DateTime};
use ethers::types::{H256, U256, U64};
use eyre::Result;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use prost::Message;
use serde::{Deserialize, Serialize};

use super::bytecode;

pub type L1BatchNumber = U64;
pub type MiniblockNumber = U64;

pub type StorageKey = U256;
pub type StorageValue = H256;

pub mod protobuf {
    include!(concat!(env!("OUT_DIR"), "/protobuf.rs"));
}

pub trait Proto {
    type ProtoStruct: Message + Default;

    /// Convert [`Self`] into its protobuf generated equivalent.
    fn to_proto(&self) -> Self::ProtoStruct;

    /// Convert from a generated protobuf struct.
    fn from_proto(proto: Self::ProtoStruct) -> Result<Self>
    where
        Self: Sized;

    /// Encode [`Self`] to file using gzip compression.
    fn encode(&self, path: &Path) -> Result<()> {
        let proto = Self::to_proto(self);

        // Ensure that write buffer has enough capacity.
        let mut buf = BytesMut::new();
        let len = proto.encoded_len();
        if buf.capacity() < len {
            buf.reserve(len - buf.capacity());
        }

        Self::ProtoStruct::encode(&proto, &mut buf)?;
        let outfile = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let mut encoder = GzEncoder::new(outfile, Compression::default());
        encoder.write_all(&buf)?;
        encoder.finish()?;

        Ok(())
    }

    /// Decode a slice of gzip-compressed bytes into [`Self`].
    fn decode(bytes: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        let mut decoder = GzDecoder::new(bytes);
        let mut decompressed_bytes = Vec::new();
        decoder.read_to_end(&mut decompressed_bytes)?;

        let proto = Self::ProtoStruct::decode(&decompressed_bytes[..])?;
        Self::from_proto(proto)
    }
}

/// Version of snapshot influencing the format of data stored in GCS.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[repr(u16)]
pub enum SnapshotVersion {
    /// Initial snapshot version. Keys in storage logs are stored as `(address, key)` pairs.
    Version0 = 0,
    /// Snapshot version made compatible with L1 recovery. Differs from `Version0` by including
    /// hashed keys in storage logs instead of `(address, key)` pairs.
    #[default]
    Version1 = 1,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SnapshotHeader {
    pub version: SnapshotVersion,
    pub l1_batch_number: L1BatchNumber,
    pub miniblock_number: MiniblockNumber,
    // ordered by chunk_id
    pub storage_logs_chunks: Vec<SnapshotStorageLogsChunkMetadata>,
    pub factory_deps_filepath: String,
    // Following `L1BatchWithMetadata` type doesn't have definition. Ignoring.
    //pub last_l1_batch_with_metadata: L1BatchWithMetadata,
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SnapshotStorageLogsChunkMetadata {
    pub chunk_id: u64,
    // can be either a gs or filesystem path
    pub filepath: String,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotStorageKey {
    pub l1_batch_number: L1BatchNumber,
    /// Chunks with smaller id's must contain `storage_logs` with smaller `hashed_keys`
    pub chunk_id: u64,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotStorageLogsChunk {
    pub storage_logs: Vec<SnapshotStorageLog>,
}

impl Proto for SnapshotStorageLogsChunk {
    type ProtoStruct = protobuf::SnapshotStorageLogsChunk;

    fn to_proto(&self) -> Self::ProtoStruct {
        Self::ProtoStruct {
            storage_logs: self
                .storage_logs
                .iter()
                .map(SnapshotStorageLog::to_proto)
                .collect(),
        }
    }

    fn from_proto(proto: Self::ProtoStruct) -> Result<Self> {
        Ok(Self {
            storage_logs: proto
                .storage_logs
                .into_iter()
                .map(SnapshotStorageLog::from_proto)
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

// "most recent" for each key together with info when the key was first used
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotStorageLog {
    pub key: StorageKey,
    pub value: StorageValue,
    pub l1_batch_number_of_initial_write: L1BatchNumber,
    pub enumeration_index: u64,
}

impl Proto for SnapshotStorageLog {
    type ProtoStruct = protobuf::SnapshotStorageLog;

    fn to_proto(&self) -> Self::ProtoStruct {
        let mut key = [0u8; 32];
        self.key.to_big_endian(&mut key);

        Self::ProtoStruct {
            account_address: None,
            storage_key: Some(key.to_vec()),
            storage_value: Some(self.value.as_bytes().to_vec()),
            l1_batch_number_of_initial_write: Some(self.l1_batch_number_of_initial_write.as_u32()),
            enumeration_index: Some(self.enumeration_index),
        }
    }

    fn from_proto(proto: Self::ProtoStruct) -> Result<Self> {
        let value_bytes: [u8; 32] = proto.storage_value().try_into()?;
        Ok(Self {
            key: U256::from_big_endian(proto.storage_key()),
            value: StorageValue::from(&value_bytes),
            l1_batch_number_of_initial_write: proto.l1_batch_number_of_initial_write().into(),
            enumeration_index: proto.enumeration_index(),
        })
    }
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotFactoryDependencies {
    pub factory_deps: Vec<SnapshotFactoryDependency>,
}

impl Proto for SnapshotFactoryDependencies {
    type ProtoStruct = protobuf::SnapshotFactoryDependencies;

    fn to_proto(&self) -> Self::ProtoStruct {
        Self::ProtoStruct {
            factory_deps: self
                .factory_deps
                .iter()
                .map(SnapshotFactoryDependency::to_proto)
                .collect(),
        }
    }

    fn from_proto(proto: Self::ProtoStruct) -> Result<Self> {
        Ok(Self {
            factory_deps: proto
                .factory_deps
                .into_iter()
                .map(SnapshotFactoryDependency::from_proto)
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct SnapshotFactoryDependency {
    pub bytecode_hash: H256,
    pub bytecode: Vec<u8>,
}

impl Proto for SnapshotFactoryDependency {
    type ProtoStruct = protobuf::SnapshotFactoryDependency;

    fn to_proto(&self) -> Self::ProtoStruct {
        Self::ProtoStruct {
            bytecode: Some(self.bytecode.clone()),
        }
    }

    fn from_proto(proto: Self::ProtoStruct) -> Result<Self> {
        let bytecode = proto.bytecode();
        Ok(Self {
            bytecode_hash: bytecode::hash_bytecode(bytecode),
            bytecode: bytecode.to_vec(),
        })
    }
}

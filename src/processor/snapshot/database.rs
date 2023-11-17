use std::{convert::TryFrom, path::PathBuf};

use ethers::types::H256;
use eyre::Result;
use rocksdb::{Options, DB};
use thiserror::Error;

use super::types::{SnapshotFactoryDependency, SnapshotStorageLog};

const STORAGE_LOGS: &str = "storage_logs";
const INDEX_TO_KEY_MAP: &str = "index_to_key_map";
const FACTORY_DEPS: &str = "factory_deps";
const METADATA: &str = "metadata";

const LAST_REPEATED_KEY_INDEX: &str = "LAST_REPEATED_KEY_INDEX";

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("no such key")]
    NoSuchKey,
}

pub struct SnapshotDB {
    db: DB,
}

impl SnapshotDB {
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let mut db_opts = Options::default();
        db_opts.create_missing_column_families(true);
        db_opts.create_if_missing(true);

        let db = DB::open_cf(
            &db_opts,
            db_path,
            vec![METADATA, STORAGE_LOGS, INDEX_TO_KEY_MAP, FACTORY_DEPS],
        )?;

        Ok(Self { db })
    }

    pub fn get_last_repeated_key_index(&self) -> Result<u64> {
        let metadata = self.db.cf_handle(METADATA).unwrap();
        Ok(
            if let Some(idx_bytes) = self.db.get_cf(metadata, LAST_REPEATED_KEY_INDEX)? {
                u64::from_be_bytes([
                    idx_bytes[0],
                    idx_bytes[1],
                    idx_bytes[2],
                    idx_bytes[3],
                    idx_bytes[4],
                    idx_bytes[5],
                    idx_bytes[6],
                    idx_bytes[7],
                ])
            } else {
                self.db
                    .put_cf(metadata, LAST_REPEATED_KEY_INDEX, u64::to_be_bytes(1))?;
                0
            },
        )
    }

    pub fn set_last_repeated_key_index(&self, idx: u64) -> Result<()> {
        let metadata = self.db.cf_handle(METADATA).unwrap();
        self.db
            .put_cf(metadata, LAST_REPEATED_KEY_INDEX, idx.to_be_bytes())
            .map_err(|e| e.into())
    }

    pub fn get_storage_log(&self, key: &[u8]) -> Result<Option<SnapshotStorageLog>> {
        let storage_logs = self.db.cf_handle(STORAGE_LOGS).unwrap();
        self.db
            .get_cf(storage_logs, key)
            .map(|v| v.map(|v| bincode::deserialize(&v).unwrap()))
            .map_err(|e| e.into())
    }

    pub fn insert_storage_log(&self, storage_log_entry: &SnapshotStorageLog) -> Result<()> {
        let index_to_key_map = self.db.cf_handle(INDEX_TO_KEY_MAP).unwrap();
        let storage_logs = self.db.cf_handle(STORAGE_LOGS).unwrap();

        let mut key: [u8; 32] = [0; 32];
        storage_log_entry.key.to_big_endian(&mut key);

        // XXX: These should really be inside a transaction...
        let idx = self.get_last_repeated_key_index()? + 1;

        self.db.put_cf(index_to_key_map, idx.to_be_bytes(), key)?;
        self.set_last_repeated_key_index(idx)?;

        self.db
            .put_cf(storage_logs, key, bincode::serialize(storage_log_entry)?)
            .map_err(|e| e.into())
    }

    pub fn update_storage_log_value(&self, key_idx: u64, value: &[u8]) -> Result<()> {
        let index_to_key_map = self.db.cf_handle(INDEX_TO_KEY_MAP).unwrap();
        let storage_logs = self.db.cf_handle(STORAGE_LOGS).unwrap();

        let key: Vec<u8> = match self.db.get_cf(index_to_key_map, key_idx.to_be_bytes())? {
            Some(k) => k,
            None => return Err(DatabaseError::NoSuchKey.into()),
        };

        // XXX: These should really be inside a transaction...
        let entry_bs = self.db.get_cf(storage_logs, &key)?.unwrap();
        let mut entry: SnapshotStorageLog = bincode::deserialize(&entry_bs)?;
        entry.value = H256::from(<&[u8; 32]>::try_from(value).unwrap());
        self.db
            .put_cf(storage_logs, key, bincode::serialize(&entry)?)
            .map_err(|e| e.into())
    }

    pub fn update_storage_log_entry(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let storage_logs = self.db.cf_handle(STORAGE_LOGS).unwrap();
        let entry_bs = self.db.get_cf(storage_logs, key)?.unwrap();
        let mut entry: SnapshotStorageLog = bincode::deserialize(&entry_bs)?;
        entry.value = H256::from(<&[u8; 32]>::try_from(value).unwrap());
        self.db
            .put_cf(storage_logs, key, bincode::serialize(&entry)?)
            .map_err(|e| e.into())
    }

    pub fn insert_factory_dep(&self, fdep: &SnapshotFactoryDependency) -> Result<()> {
        let factory_deps = self.db.cf_handle(FACTORY_DEPS).unwrap();
        self.db
            .put_cf(factory_deps, fdep.bytecode_hash, bincode::serialize(&fdep)?)
            .map_err(|e| e.into())
    }
}

use std::{ops::Deref, path::PathBuf};

use ethers::types::{H256, U256, U64};
use eyre::Result;
use rocksdb::{Options, DB};

use crate::{
    snapshot_columns,
    types::{SnapshotFactoryDependency, SnapshotStorageLog},
    DatabaseError, PackingType, INDEX_TO_KEY_MAP, KEY_TO_INDEX_MAP, METADATA,
};

pub struct SnapshotDatabase(DB);

impl Deref for SnapshotDatabase {
    type Target = DB;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SnapshotDatabase {
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let mut db_opts = Options::default();
        db_opts.create_missing_column_families(true);
        db_opts.create_if_missing(true);
        db_opts.set_max_open_files(1024);

        let db = DB::open_cf(
            &db_opts,
            db_path,
            vec![
                METADATA,
                INDEX_TO_KEY_MAP,
                KEY_TO_INDEX_MAP,
                snapshot_columns::STORAGE_LOGS,
                snapshot_columns::FACTORY_DEPS,
            ],
        )?;

        Ok(Self(db))
    }

    pub fn new_read_only(db_path: PathBuf) -> Result<Self> {
        let mut db_opts = Options::default();
        db_opts.create_missing_column_families(true);
        db_opts.create_if_missing(true);

        let db = DB::open_cf_for_read_only(
            &db_opts,
            db_path,
            vec![
                METADATA,
                INDEX_TO_KEY_MAP,
                KEY_TO_INDEX_MAP,
                snapshot_columns::STORAGE_LOGS,
                snapshot_columns::FACTORY_DEPS,
            ],
            false,
        )?;

        Ok(Self(db))
    }

    pub fn process_value(&self, key: U256, value: PackingType) -> Result<H256> {
        let processed_value = match value {
            PackingType::NoCompression(v) | PackingType::Transform(v) => v,
            PackingType::Add(_) | PackingType::Sub(_) => {
                let existing_value = if let Some(log) = self.get_storage_log(&key)? {
                    U256::from(log.value.to_fixed_bytes())
                } else {
                    U256::from(0)
                };

                // NOTE: We're explicitly allowing over-/underflow as per the spec.
                match value {
                    PackingType::Add(v) => existing_value.overflowing_add(v).0,
                    PackingType::Sub(v) => existing_value.overflowing_sub(v).0,
                    _ => unreachable!(),
                }
            }
        };

        let mut buffer = [0; 32];
        processed_value.to_big_endian(&mut buffer);
        Ok(H256::from(buffer))
    }

    pub fn get_storage_log(&self, key: &U256) -> Result<Option<SnapshotStorageLog>> {
        // Unwrapping column family handle here is safe because presence of
        // those CFs is ensured in construction of this DB.
        let storage_logs = self.cf_handle(snapshot_columns::STORAGE_LOGS).unwrap();
        let mut byte_key = [0u8; 32];
        key.to_big_endian(&mut byte_key);
        self.get_cf(storage_logs, byte_key)
            .map(|v| v.map(|v| bincode::deserialize(&v).unwrap()))
            .map_err(Into::into)
    }

    pub fn insert_storage_log(&mut self, storage_log_entry: &mut SnapshotStorageLog) -> Result<()> {
        // Unwrapping column family handle here is safe because presence of
        // those CFs is ensured in construction of this DB.
        let index_to_key_map = self.cf_handle(INDEX_TO_KEY_MAP).unwrap();
        let storage_logs = self.cf_handle(snapshot_columns::STORAGE_LOGS).unwrap();

        let mut key: [u8; 32] = [0; 32];
        storage_log_entry.key.to_big_endian(&mut key);

        // XXX: These should really be inside a transaction...
        let idx = self.get_last_repeated_key_index()? + 1;

        // Update the enumeration index.
        storage_log_entry.enumeration_index = idx;

        self.put_cf(index_to_key_map, idx.to_be_bytes(), key)?;
        self.set_last_repeated_key_index(idx)?;

        self.put_cf(storage_logs, key, bincode::serialize(storage_log_entry)?)
            .map_err(Into::into)
    }

    pub fn get_key_from_index(&self, key_idx: u64) -> Result<Vec<u8>> {
        let index_to_key_map = self.cf_handle(INDEX_TO_KEY_MAP).unwrap();
        match self.get_cf(index_to_key_map, key_idx.to_be_bytes())? {
            Some(k) => Ok(k),
            None => Err(DatabaseError::NoSuchKey.into()),
        }
    }

    pub fn update_storage_log_value(&self, key_idx: u64, value: &[u8]) -> Result<()> {
        // Unwrapping column family handle here is safe because presence of
        // those CFs is ensured in construction of this DB.
        let storage_logs = self.cf_handle(snapshot_columns::STORAGE_LOGS).unwrap();
        let key = self.get_key_from_index(key_idx)?;

        // XXX: These should really be inside a transaction...
        let entry_bs = self.get_cf(storage_logs, &key)?.unwrap();
        let mut entry: SnapshotStorageLog = bincode::deserialize(&entry_bs)?;
        entry.value = H256::from(<&[u8; 32]>::try_from(value).unwrap());
        self.put_cf(storage_logs, key, bincode::serialize(&entry)?)
            .map_err(Into::into)
    }

    pub fn insert_factory_dep(&self, fdep: &SnapshotFactoryDependency) -> Result<()> {
        // Unwrapping column family handle here is safe because presence of
        // those CFs is ensured in construction of this DB.
        let factory_deps = self.cf_handle(snapshot_columns::FACTORY_DEPS).unwrap();
        self.put_cf(factory_deps, fdep.bytecode_hash, bincode::serialize(&fdep)?)
            .map_err(Into::into)
    }

    pub fn get_latest_l1_batch_number(&self) -> Result<U64> {
        self.get_metadata_value(snapshot_columns::LATEST_L1_BATCH)
            .map(U64::from)
    }

    pub fn set_latest_l1_batch_number(&self, number: u64) -> Result<()> {
        self.set_metadata_value(snapshot_columns::LATEST_L1_BATCH, number)
    }

    pub fn get_last_repeated_key_index(&self) -> Result<u64> {
        self.get_metadata_value(snapshot_columns::LAST_REPEATED_KEY_INDEX)
    }

    pub fn set_last_repeated_key_index(&self, idx: u64) -> Result<()> {
        self.set_metadata_value(snapshot_columns::LAST_REPEATED_KEY_INDEX, idx)
    }

    fn get_metadata_value(&self, value_name: &str) -> Result<u64> {
        let metadata = self.cf_handle(METADATA).unwrap();
        Ok(
            if let Some(idx_bytes) = self.get_cf(metadata, value_name)? {
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
                0
            },
        )
    }

    fn set_metadata_value(&self, value_name: &str, value: u64) -> Result<()> {
        let metadata = self.cf_handle(METADATA).unwrap();
        self.put_cf(metadata, value_name, value.to_be_bytes())
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const TEST_DB_PATH: &str = "reconstruction_test_db";

    #[test]
    fn basics() {
        let db_dir = PathBuf::from(TEST_DB_PATH);
        {
            let db = SnapshotDatabase::new(db_dir.clone()).unwrap();
            let zero = db.get_last_repeated_key_index().unwrap();
            assert_eq!(zero, 0);
            db.set_last_repeated_key_index(1).unwrap();
            let one = db.get_last_repeated_key_index().unwrap();
            assert_eq!(one, 1);
        }
        fs::remove_dir_all(db_dir).unwrap();
    }
}

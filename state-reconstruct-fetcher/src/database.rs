use std::{convert::Into, ops::Deref, path::PathBuf};

use ethers::types::{U256, U64};
use eyre::Result;
use rocksdb::{Options, DB};
use thiserror::Error;

const INDEX_TO_KEY_MAP: &str = "index_to_key_map";
const KEY_TO_INDEX_MAP: &str = "key_to_index_map";
const METADATA: &str = "metadata";

const LAST_REPEATED_KEY_INDEX: &str = "LAST_REPEATED_KEY_INDEX";
/// The latest l1 block number that was processed.
const LATEST_L1_BLOCK_NUMBER: &str = "LATEST_L1_BLOCK_NUMBER";
/// The latest l2 block number that was processed.
const LATEST_L2_BLOCK_NUMBER: &str = "LATEST_L2_BLOCK_NUMBER";

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("key not found")]
    NoSuchKey,
}

pub struct InnerDB(DB);

impl Deref for InnerDB {
    type Target = DB;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl InnerDB {
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let mut db_opts = Options::default();
        db_opts.create_missing_column_families(true);
        db_opts.create_if_missing(true);

        let db = DB::open_cf(
            &db_opts,
            db_path,
            vec![METADATA, INDEX_TO_KEY_MAP, KEY_TO_INDEX_MAP],
        )?;

        Ok(Self(db))
    }

    pub fn get_latest_l1_block_number(&self) -> Result<U64> {
        self.get_metadata_value(LATEST_L1_BLOCK_NUMBER)
            .map(U64::from)
    }

    pub fn set_latest_l1_block_number(&self, number: u64) -> Result<()> {
        self.set_metadata_value(LATEST_L1_BLOCK_NUMBER, number)
    }

    pub fn get_latest_l2_block_number(&self) -> Result<u64> {
        self.get_metadata_value(LATEST_L2_BLOCK_NUMBER)
    }

    pub fn set_latest_l2_block_number(&self, number: u64) -> Result<()> {
        self.set_metadata_value(LATEST_L2_BLOCK_NUMBER, number)
    }

    pub fn get_last_repeated_key_index(&self) -> Result<u64> {
        self.get_metadata_value(LAST_REPEATED_KEY_INDEX)
    }

    pub fn set_last_repeated_key_index(&self, idx: u64) -> Result<()> {
        self.set_metadata_value(LAST_REPEATED_KEY_INDEX, idx)
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

    pub fn get_key(&self, idx: u64) -> Result<U256> {
        let idx2key = self.cf_handle(INDEX_TO_KEY_MAP).unwrap();
        let idx_bytes = idx.to_be_bytes();
        if let Some(key_bytes) = self.get_cf(idx2key, idx_bytes)? {
            Ok(U256::from_big_endian(&key_bytes))
        } else {
            Err(DatabaseError::NoSuchKey.into())
        }
    }

    pub fn add_key(&self, key: &U256) -> Result<()> {
        let mut key_bytes: [u8; 32] = [0; 32];
        key.to_big_endian(&mut key_bytes);

        let key2idx = self.cf_handle(KEY_TO_INDEX_MAP).unwrap();
        // FIXME: These should be inside a transaction...
        if let Some(_idx_bytes) = self.get_cf(key2idx, key_bytes)? {
            return Ok(());
        }

        let idx2key = self.cf_handle(INDEX_TO_KEY_MAP).unwrap();
        let idx = self.get_last_repeated_key_index()?;
        let idx_bytes = idx.to_be_bytes();
        self.put_cf(idx2key, idx_bytes, key_bytes)?;
        self.put_cf(key2idx, key_bytes, idx_bytes)?;
        self.set_last_repeated_key_index(idx + 1)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn basics() {
        let db_dir = PathBuf::from("./test_inner_db");
        {
            let db = InnerDB::new(db_dir.clone()).unwrap();
            let zero = db.get_last_repeated_key_index().unwrap();
            assert_eq!(zero, 0);
            db.set_last_repeated_key_index(1).unwrap();
            let one = db.get_last_repeated_key_index().unwrap();
            assert_eq!(one, 1);
        }
        fs::remove_dir_all(db_dir).unwrap();
    }
}

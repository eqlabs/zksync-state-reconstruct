use std::{fmt, path::Path};

use eyre::Result;
use serde::Serialize;
use zksync_merkle_tree::{MerkleTree, RocksDBWrapper};

use crate::cli::Query;

#[derive(Serialize)]
pub struct RootHashQuery {
    pub batch: u64,
    pub root_hash: String,
}

impl fmt::Display for RootHashQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Batch: {}\nRoot Hash: {}", self.batch, self.root_hash)
    }
}

pub struct QueryTree(MerkleTree<RocksDBWrapper>);

impl QueryTree {
    pub fn new(db_path: &Path) -> Result<Self> {
        assert!(db_path.exists());

        let db = RocksDBWrapper::new(db_path)?;
        let tree = MerkleTree::new(db);

        Ok(Self(tree))
    }

    pub fn query(&self, query: &Query) -> RootHashQuery {
        match query {
            Query::RootHash => self.query_root_hash(),
        }
    }

    fn query_root_hash(&self) -> RootHashQuery {
        RootHashQuery {
            // Note that the L2 batch number will diverge from what's
            // published on L1 in case `TreeWrapper::insert_block`
            // fails (i.e. the program ends with Root hash mismatch
            // error).
            batch: self.0.latest_version().unwrap_or_default(),
            root_hash: hex::encode(self.0.latest_root_hash()),
        }
    }
}

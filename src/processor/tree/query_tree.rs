use std::path::Path;

use zksync_merkle_tree::{MerkleTree, RocksDBWrapper};

use super::RootHash;

pub struct QueryTree<'a>(MerkleTree<'a, RocksDBWrapper>);

impl QueryTree<'static> {
    pub fn new(db_path: &Path) -> Self {
        assert!(db_path.exists());

        let db = RocksDBWrapper::new(db_path);
        let tree = MerkleTree::new(db);

        Self(tree)
    }

    pub fn latest_root_hash(&self) -> RootHash {
        self.0.latest_root_hash()
    }
}

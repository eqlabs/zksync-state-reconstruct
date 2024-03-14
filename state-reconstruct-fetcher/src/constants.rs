pub mod ethereum {
    /// Number of Ethereum blocks to advance in one filter step.
    pub const BLOCK_STEP: u64 = 128;

    /// Block number in Ethereum for zkSync genesis block.
    pub const GENESIS_BLOCK: u64 = 16_627_460;

    /// Block number in Ethereum of the first Boojum-formatted block.
    pub const BOOJUM_BLOCK: u64 = 18_715_403;

    /// Block number in Ethereum of the first block storing pubdata within blobs.
    pub const BLOB_BLOCK: u64 = 19_412_473;

    /// zkSync smart contract address.
    pub const ZK_SYNC_ADDR: &str = "0x32400084C286CF3E17e7B677ea9583e60a000324";

    /// Default Ethereum blob storage URL base.
    pub const BLOBS_URL: &str = "https://api.blobscan.com/blobs/";
}

pub mod storage {
    /// The path to the initial state file.
    pub const INITAL_STATE_PATH: &str = "InitialState.csv";

    /// The default name of the database.
    pub const DEFAULT_DB_NAME: &str = "db";

    /// The name of the index-to-key database folder.
    pub const INNER_DB_NAME: &str = "inner_db";
}

pub mod zksync {
    /// Bytes in raw L2 to L1 log.
    pub const L2_TO_L1_LOG_SERIALIZE_SIZE: usize = 88;
    // The bitmask by applying which to the compressed state diff metadata we retrieve its operation.
    pub const OPERATION_BITMASK: u8 = 7;
    // The number of bits shifting the compressed state diff metadata by which we retrieve its length.
    pub const LENGTH_BITS_OFFSET: u8 = 3;
    // Size of CommitBatchInfo.pubdataCommitments item.
    pub const PUBDATA_COMMITMENT_SIZE: usize = 144;
}

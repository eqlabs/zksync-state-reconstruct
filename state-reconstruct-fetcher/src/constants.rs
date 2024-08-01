pub mod ethereum {
    /// Number of Ethereum blocks to advance in one filter step.
    pub const BLOCK_STEP: u64 = 10_000;

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
    /// The bitmask by applying which to the compressed state diff metadata we retrieve its operation.
    pub const OPERATION_BITMASK: u8 = 7;
    /// The number of bits shifting the compressed state diff metadata by which we retrieve its length.
    pub const LENGTH_BITS_OFFSET: u8 = 3;
    /// Size of `CommitBatchInfo.pubdataCommitments` item.
    pub const PUBDATA_COMMITMENT_SIZE: usize = 144;
    /// The number of trailing bytes to ignore when using calldata post-blobs. Contains unused blob commitments.
    pub const CALLDATA_SOURCE_TAIL_SIZE: usize = 32;

    /// NOTE: There could be more but this covers a good chunk.
    /// The storage addresses where the latest L2 block number is written to.
    pub const L2_BLOCK_NUMBER_ADDRESSES: [&str; 2] = [
        "5e5a67d1b864c576f39bb2b77c6537744c0f03515abce63b473bb7c56ad07d8e",
        "ecfc4e86b2e01c263feada4f8f53d2dab45c66b0f4d1d7ab0f2f8ec32f207c48",
    ];
}

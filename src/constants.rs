pub mod ethereum {
    /// Number of Ethereum blocks to advance in one filter step.
    pub const BLOCK_STEP: u64 = 128;

    /// Block number in Ethereum for zkSync genesis block.
    pub const GENESIS_BLOCK: u64 = 16_627_460;

    /// zkSync smart contract address.
    pub const ZK_SYNC_ADDR: &str = "0x32400084C286CF3E17e7B677ea9583e60a000324";
}

pub mod storage {
    /// The path to the initial state file.
    pub const INITAL_STATE_PATH: &str = "InitialState.csv";

    /// The default name of the database.
    pub const DEFAULT_DB_NAME: &str = "db";

    /// The name of the state file.
    pub const STATE_FILE_NAME: &str = "StateSnapshot.json";
}

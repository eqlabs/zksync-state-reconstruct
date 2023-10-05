use clap::{Parser, Subcommand};

use crate::constants::ethereum;

#[derive(Subcommand)]
pub enum ReconstructSource {
    /// Fetch data from L1.
    L1 {
        /// The Ethereum JSON-RPC HTTP URL to use.
        #[arg(long)]
        http_url: String,
        /// Ethereum block number to start state import from.
        #[arg(short, long, default_value_t=ethereum::GENESIS_BLOCK)]
        start_block: u64,
        /// The number of blocks to filter & process in one step over.
        #[arg(short, long, default_value_t=ethereum::BLOCK_STEP)]
        block_step: u64,
    },
    /// Fetch data from a file.
    File {
        /// The path of the file to import state from.
        #[arg(short, long)]
        file: String,
    },
}

#[derive(Subcommand)]
pub enum Commands {
    /// Reconstruct L2 state from a source.
    #[command(subcommand)]
    Reconstruct(ReconstructSource),
}

#[derive(Parser)]
#[command(author, version, about = "zkSync state reconstruction tool")]
pub struct Args {
    #[command(subcommand)]
    pub subcommand: Commands,
}

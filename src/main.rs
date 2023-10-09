#![feature(array_chunks)]

mod cli;
mod constants;
mod l1_fetcher;
mod processor;
mod types;

use std::{
    env,
    path::{Path, PathBuf},
};

use clap::Parser;
use cli::*;
use constants::storage;
use ethers::types::U64;
use eyre::Result;
use l1_fetcher::L1Fetcher;
use tokio::sync::mpsc;

use crate::{
    processor::{json::JsonSerializationProcessor, tree::TreeProcessor, Processor},
    types::CommitBlockInfoV1,
};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.subcommand {
        Commands::Reconstruct { source, db_path } => {
            let db_path = match db_path {
                Some(path) => PathBuf::from(path),
                None => env::current_dir()?.join(storage::DEFAULT_DB_NAME),
            };

            match source {
                ReconstructSource::L1 {
                    http_url,
                    start_block,
                    block_step: _,
                } => {
                    let fetcher = L1Fetcher::new(&http_url)?;
                    let processor = TreeProcessor::new(db_path)?;
                    let (tx, rx) = mpsc::channel::<Vec<CommitBlockInfoV1>>(5);

                    tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    fetcher.fetch(tx, Some(U64([start_block])), None).await?;
                }
                ReconstructSource::File { file: _ } => todo!(),
            }
        }
        Commands::Download {
            http_url,
            start_block,
            block_step: _,
            block_count,
            file,
        } => {
            let fetcher = L1Fetcher::new(&http_url)?;
            let processor = JsonSerializationProcessor::new(Path::new(&file))?;
            let (tx, rx) = mpsc::channel::<Vec<CommitBlockInfoV1>>(5);

            tokio::spawn(async move {
                processor.run(rx).await;
            });

            let end_block = match block_count {
                Some(n) => Some(U64([start_block + n])),
                None => None,
            };

            fetcher
                .fetch(tx, Some(U64([start_block])), end_block)
                .await?;
        }
        _ => unreachable!(),
    }

    Ok(())
}

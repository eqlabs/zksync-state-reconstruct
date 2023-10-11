#![feature(array_chunks)]
#![feature(iter_next_chunk)]

mod cli;
mod constants;
mod json;
mod l1_fetcher;
mod processor;
mod types;

use std::{
    env,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use clap::Parser;
use cli::{Cli, Command, L1FetcherOptions, Query, ReconstructSource};
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
    let cli = Cli::parse();

    match cli.subcommand {
        Command::Reconstruct { source, db_path } => {
            let db_path = match db_path {
                Some(path) => PathBuf::from(path),
                None => env::current_dir()?.join(storage::DEFAULT_DB_NAME),
            };

            match source {
                ReconstructSource::L1 {
                    l1_fetcher_options:
                        L1FetcherOptions {
                            http_url,
                            start_block,
                            block_step: _,
                            block_count: _,
                        },
                } => {
                    let fetcher = L1Fetcher::new(&http_url)?;
                    let processor = TreeProcessor::new(db_path)?;
                    let (tx, rx) = mpsc::channel::<CommitBlockInfoV1>(5);

                    tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    fetcher.fetch(tx, Some(U64([start_block])), None).await?;
                }
                ReconstructSource::File { file } => {
                    let reader = BufReader::new(File::open(&file)?);
                    let processor = TreeProcessor::new(db_path)?;
                    let (tx, rx) = mpsc::channel::<CommitBlockInfoV1>(5);

                    tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    let json_iter = json::iter_json_array::<CommitBlockInfoV1, _>(reader);
                    let mut num_objects = 0;
                    for blk in json_iter {
                        tx.send(blk.expect("parsing")).await?;
                        num_objects += 1;
                    }

                    println!("{num_objects} objects imported from {file}");
                }
            }
        }
        Command::Download {
            l1_fetcher_options:
                L1FetcherOptions {
                    http_url,
                    start_block,
                    block_step: _,
                    block_count,
                },
            file,
        } => {
            let fetcher = L1Fetcher::new(&http_url)?;
            let processor = JsonSerializationProcessor::new(Path::new(&file))?;
            let (tx, rx) = mpsc::channel::<CommitBlockInfoV1>(5);

            tokio::spawn(async move {
                processor.run(rx).await;
            });

            let end_block = block_count.map(|n| U64([start_block + n]));

            fetcher
                .fetch(tx, Some(U64([start_block])), end_block)
                .await?;
        }
        Command::Query {
            query,
            json,
            db_path,
        } => {
            let db_path = match db_path {
                Some(path) => PathBuf::from(path),
                None => env::current_dir()?.join(storage::DEFAULT_DB_NAME),
            };

            let processor = TreeProcessor::new(db_path)?;
            let result = match query {
                Query::RootHash => processor.tree.latest_root_hash(),
            };

            if json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!("{result}");
            }
        }
    }

    Ok(())
}

#![feature(array_chunks)]
#![feature(iter_next_chunk)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

mod cli;
mod constants;
mod l1_fetcher;
mod processor;
mod snapshot;
mod types;
mod util;

use std::{
    env,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Parser;
use cli::{Cli, Command, L1FetcherOptions, Query, ReconstructSource};
use constants::storage;
use ethers::types::U64;
use eyre::Result;
use snapshot::StateSnapshot;
use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

use crate::{
    l1_fetcher::L1Fetcher,
    processor::{
        json::JsonSerializationProcessor,
        tree::{query_tree::QueryTree, TreeProcessor},
        Processor,
    },
    types::CommitBlockInfoV1,
    util::json,
};

fn start_logger(default_level: LevelFilter) {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("ethers=off".parse().unwrap()),
        _ => EnvFilter::default()
            .add_directive(default_level.into())
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("ethers=off".parse().unwrap()),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    start_logger(LevelFilter::INFO);

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
                            block_count,
                            disable_polling,
                        },
                } => {
                    let snapshot = Arc::new(Mutex::new(StateSnapshot::default()));

                    let fetcher = L1Fetcher::new(&http_url, Some(snapshot.clone()))?;
                    let processor = TreeProcessor::new(db_path, snapshot.clone()).await?;
                    let (tx, rx) = mpsc::channel::<CommitBlockInfoV1>(5);

                    let processor_handle = tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    let end_block = block_count.map(|n| U64([start_block + n]));

                    fetcher
                        .fetch(tx, Some(U64([start_block])), end_block, disable_polling)
                        .await?;
                    processor_handle.await?;
                }
                ReconstructSource::File { file } => {
                    let snapshot = Arc::new(Mutex::new(StateSnapshot::default()));

                    let reader = BufReader::new(File::open(&file)?);
                    let processor = TreeProcessor::new(db_path, snapshot).await?;
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

                    tracing::info!("{num_objects} objects imported from {file}");
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
                    disable_polling,
                },
            file,
        } => {
            let fetcher = L1Fetcher::new(&http_url, None)?;
            let processor = JsonSerializationProcessor::new(Path::new(&file))?;
            let (tx, rx) = mpsc::channel::<CommitBlockInfoV1>(5);

            let processor_handle = tokio::spawn(async move {
                processor.run(rx).await;
            });

            let end_block = block_count.map(|n| U64([start_block + n]));

            fetcher
                .fetch(tx, Some(U64([start_block])), end_block, disable_polling)
                .await?;
            processor_handle.await?;
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

            let tree = QueryTree::new(&db_path);
            let result = match query {
                Query::RootHash => tree.latest_root_hash(),
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

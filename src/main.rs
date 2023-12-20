#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

mod cli;
mod processor;
mod util;

use std::{
    env,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Parser;
use cli::{Cli, Command, ReconstructSource};
use eyre::Result;
use processor::snapshot::{SnapshotBuilder, SnapshotExporter};
use state_reconstruct_fetcher::{
    constants::storage::{self, STATE_FILE_NAME},
    l1_fetcher::{L1Fetcher, L1FetcherOptions},
    snapshot::StateSnapshot,
    types::CommitBlock,
};
use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

use crate::{
    processor::{
        json::JsonSerializationProcessor,
        tree::{query_tree::QueryTree, TreeProcessor},
        Processor,
    },
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
#[allow(clippy::too_many_lines)]
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
                ReconstructSource::L1 { l1_fetcher_options } => {
                    let snapshot = Arc::new(Mutex::new(StateSnapshot::default()));
                    let fetcher_options = L1FetcherOptions {
                        http_url: l1_fetcher_options.http_url,
                        start_block: l1_fetcher_options.start_block,
                        block_step: l1_fetcher_options.block_step,
                        block_count: l1_fetcher_options.block_count,
                        disable_polling: l1_fetcher_options.disable_polling,
                    };

                    let fetcher = L1Fetcher::new(fetcher_options, Some(snapshot.clone()))?;
                    let processor = TreeProcessor::new(db_path.clone(), snapshot.clone()).await?;
                    let (tx, rx) = mpsc::channel::<CommitBlock>(5);

                    let processor_handle = tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    fetcher.run(tx).await?;
                    processor_handle.await?;

                    // Write the current state to a file.
                    let snapshot = snapshot.lock().await;
                    let state_file_path = db_path.join(STATE_FILE_NAME);
                    snapshot.write(&state_file_path)?;
                }
                ReconstructSource::File { file } => {
                    let snapshot = Arc::new(Mutex::new(StateSnapshot::default()));

                    let reader = BufReader::new(File::open(&file)?);
                    let processor = TreeProcessor::new(db_path, snapshot).await?;
                    let (tx, rx) = mpsc::channel::<CommitBlock>(5);

                    tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    let json_iter = json::iter_json_array::<CommitBlock, _>(reader);
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
            l1_fetcher_options,
            file,
        } => {
            let fetcher_options = L1FetcherOptions {
                http_url: l1_fetcher_options.http_url,
                start_block: l1_fetcher_options.start_block,
                block_step: l1_fetcher_options.block_step,
                block_count: l1_fetcher_options.block_count,
                disable_polling: l1_fetcher_options.disable_polling,
            };

            let fetcher = L1Fetcher::new(fetcher_options, None)?;
            let processor = JsonSerializationProcessor::new(Path::new(&file))?;
            let (tx, rx) = mpsc::channel::<CommitBlock>(5);

            let processor_handle = tokio::spawn(async move {
                processor.run(rx).await;
            });

            fetcher.run(tx).await?;
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
            let result = tree.query(&query);

            if json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!("{result}");
            }
        }
        Command::PrepareSnapshot {
            l1_fetcher_options,
            db_path,
        } => {
            let fetcher_options = L1FetcherOptions {
                http_url: l1_fetcher_options.http_url,
                start_block: l1_fetcher_options.start_block,
                block_step: l1_fetcher_options.block_step,
                block_count: l1_fetcher_options.block_count,
                disable_polling: l1_fetcher_options.disable_polling,
            };

            let fetcher = L1Fetcher::new(fetcher_options, None)?;
            let processor = SnapshotBuilder::new(db_path);

            let (tx, rx) = mpsc::channel::<CommitBlock>(5);
            let processor_handle = tokio::spawn(async move {
                processor.run(rx).await;
            });

            fetcher.run(tx).await?;
            processor_handle.await?;
        }
        Command::ExportSnapshot {
            db_path,
            chunk_size,
            directory,
        } => {
            let export_path = Path::new(&directory);
            std::fs::create_dir_all(export_path)?;
            let exporter = SnapshotExporter::new(export_path, db_path);
            exporter.export_snapshot(chunk_size)?;
        }
    }

    Ok(())
}

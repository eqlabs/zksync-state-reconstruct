#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

mod cli;
mod processor;
mod util;

use std::{
    env,
    ffi::CStr,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use clap::Parser;
use cli::{Cli, Command, ReconstructSource};
use eyre::Result;
use processor::snapshot::{SnapshotBuilder, SnapshotExporter};
use state_reconstruct_fetcher::{
    constants::storage,
    l1_fetcher::{L1Fetcher, L1FetcherOptions},
    types::CommitBlock,
};
use syslog_tracing::Syslog;
use tokio::sync::mpsc;
use tracing_subscriber::{filter::LevelFilter, prelude::*, registry::Registry, EnvFilter};

use crate::{
    processor::{
        json::JsonSerializationProcessor,
        tree::{query_tree::QueryTree, TreeProcessor},
        Processor,
    },
    util::json,
};

fn start_logger(default_level: LevelFilter, with_syslog: bool) {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        _ => EnvFilter::default().add_directive(default_level.into()),
    };
    let filter = filter
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("ethers=off".parse().unwrap());

    let default_layer = tracing_subscriber::fmt::layer().with_target(false);

    let subscriber = Registry::default().with(filter).with(default_layer);

    if with_syslog {
        let identity = CStr::from_bytes_with_nul(b"zksync-sr\0").unwrap();
        let (options, facility) = Default::default();
        let syslog = Syslog::new(identity, options, facility).unwrap();
        let syslog_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_ansi(false)
            .without_time() // syslog logs its own time
            .with_writer(syslog);

        subscriber.with(syslog_layer).init();
    } else {
        subscriber.init();
    }
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    start_logger(LevelFilter::INFO, cli.with_syslog);

    match cli.subcommand {
        Command::Reconstruct { source, db_path } => {
            let db_path = match db_path {
                Some(path) => PathBuf::from(path),
                None => env::current_dir()?.join(storage::DEFAULT_DB_NAME),
            };

            match source {
                ReconstructSource::L1 { l1_fetcher_options } => {
                    let fetcher_options = L1FetcherOptions {
                        http_url: l1_fetcher_options.http_url,
                        start_block: l1_fetcher_options.start_block,
                        block_step: l1_fetcher_options.block_step,
                        block_count: l1_fetcher_options.block_count,
                        disable_polling: l1_fetcher_options.disable_polling,
                    };

                    let processor = TreeProcessor::new(db_path.clone()).await?;
                    let fetcher = L1Fetcher::new(fetcher_options, Some(processor.get_snapshot()))?;
                    let (tx, rx) = mpsc::channel::<CommitBlock>(5);

                    let processor_handle = tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    fetcher.run(tx).await?;
                    processor_handle.await?;
                }
                ReconstructSource::File { file } => {
                    let reader = BufReader::new(File::open(&file)?);
                    let processor = TreeProcessor::new(db_path).await?;
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

            tracing::info!("Successfully downloaded CommitBlocks to {}", file);
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

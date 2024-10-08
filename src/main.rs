#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

mod cli;
mod processor;

use std::{
    env,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use ::eyre::Result;
use clap::Parser;
use cli::{Cli, Command, ReconstructSource};
use processor::snapshot::{
    exporter::SnapshotExporter, importer::SnapshotImporter, SnapshotBuilder,
};
use state_reconstruct_fetcher::{
    constants::{ethereum, storage},
    l1_fetcher::{L1Fetcher, L1FetcherOptions},
    types::CommitBlock,
};
use state_reconstruct_utils::json;
use tikv_jemallocator::Jemalloc;
use tokio::sync::mpsc;
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

use crate::processor::{
    json::JsonSerializationProcessor,
    tree::{query_tree::QueryTree, TreeProcessor},
    Processor,
};

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

fn start_logger(default_level: LevelFilter) {
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("ethers=off".parse().unwrap()),
        _ => EnvFilter::default()
            .add_directive(default_level.into())
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("ethers=off".parse().unwrap())
            .add_directive("zksync_storage=off".parse().unwrap()),
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
        Command::Reconstruct {
            source,
            db_path,
            snapshot,
        } => {
            let db_path = match db_path {
                Some(path) => PathBuf::from(path),
                None => env::current_dir()?.join(storage::DEFAULT_DB_NAME),
            };

            let snapshot_end_batch = match snapshot {
                Some(directory) => {
                    tracing::info!("Trying to restore state from snapshot...");
                    let importer = SnapshotImporter::new(PathBuf::from(directory));
                    let end_batch = importer.run(&db_path.clone()).await?;
                    Some(end_batch)
                }
                None => None,
            };

            match source {
                ReconstructSource::L1 { l1_fetcher_options } => {
                    let fetcher_options = l1_fetcher_options.into();
                    let processor = TreeProcessor::new(db_path.clone()).await?;
                    let fetcher = L1Fetcher::new(fetcher_options, Some(processor.get_inner_db()))?;

                    let (tx, rx) = mpsc::channel::<CommitBlock>(5);

                    let processor_handle = tokio::spawn(async move {
                        processor.run(rx).await;
                    });

                    fetcher.run(tx, snapshot_end_batch).await?;
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
            let fetcher_options = l1_fetcher_options.into();
            let fetcher = L1Fetcher::new(fetcher_options, None)?;
            let processor = JsonSerializationProcessor::new(Path::new(&file))?;
            let (tx, rx) = mpsc::channel::<CommitBlock>(5);

            let processor_handle = tokio::spawn(async move {
                processor.run(rx).await;
            });

            fetcher.run(tx, None).await?;
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

            let tree = QueryTree::new(&db_path)?;
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
            let processor = SnapshotBuilder::new(db_path);

            let mut fetcher_options: L1FetcherOptions = l1_fetcher_options.into();
            if let Ok(batch_number) = processor.get_latest_l1_block_number() {
                let batch_number = batch_number.as_u64();
                if batch_number > ethereum::GENESIS_BLOCK {
                    tracing::info!(
                        "Found a preexisting snapshot db, continuing from L1 block: {batch_number}"
                    );
                    fetcher_options.start_block = batch_number + 1;
                }
            }

            let fetcher = L1Fetcher::new(fetcher_options, None)?;

            let (tx, rx) = mpsc::channel::<CommitBlock>(5);
            let processor_handle = tokio::spawn(async move {
                processor.run(rx).await;
            });

            fetcher.run(tx, None).await?;
            processor_handle.await?;
        }
        Command::ExportSnapshot { db_path, directory } => {
            let export_path = Path::new(&directory);
            std::fs::create_dir_all(export_path)?;
            let exporter = SnapshotExporter::new(export_path, db_path)?;
            exporter.export_snapshot()?;

            tracing::info!("Succesfully exported snapshot files to \"{directory}\"!");
        }
    }

    Ok(())
}

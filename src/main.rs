#![feature(array_chunks)]

mod constants;
mod l1_fetcher;
mod processor;
mod types;

use std::env;

use clap::{arg, value_parser, Command};
use constants::ethereum::{BLOCK_STEP, GENESIS_BLOCK};
use ethers::types::U64;
use eyre::Result;
use l1_fetcher::L1Fetcher;
use tokio::sync::mpsc;

use crate::{
    processor::{tree::TreeProcessor, Processor},
    types::CommitBlockInfoV1,
};

fn cli() -> Command {
    Command::new("state-reconstruct")
        .about("zkSync state reconstruction tool")
        .subcommand_required(true)
        .arg_required_else_help(false)
        .subcommand(
            Command::new("reconstruct")
                .about("Reconstruct L2 state")
                .subcommand_required(true)
                .subcommand(
                    Command::new("l1")
                        .about("Read state from Ethereum L1")
                        .arg(arg!(--"http-url" <HTTP_URL>).help("Ethereum JSON-RPC HTTP URL"))
                        .arg(
                            arg!(--"start-block" <START_BLOCK>)
                                .help("Ethereum block number to start state import from")
                                .default_value(GENESIS_BLOCK.to_string())
                                .value_parser(value_parser!(u64)),
                        )
                        .arg(
                            arg!(--"block-step" <BLOCK_STEP>)
                                .help("Number of blocks to filter & process in one step")
                                .default_value(BLOCK_STEP.to_string())
                                .value_parser(value_parser!(u64)),
                        ),
                )
                .subcommand(
                    Command::new("file")
                        .about("Read state from file")
                        .arg(arg!(<FILE> "File to import state from"))
                        .arg_required_else_help(true),
                ),
        )
}

#[tokio::main]
async fn main() -> Result<()> {
    let matches = cli().get_matches();

    match matches.subcommand() {
        Some(("reconstruct", sub_matches)) => match sub_matches.subcommand() {
            Some(("l1", args)) => {
                // TODO: Use start_block from snapshot.
                let start_block = args.get_one::<u64>("start-block").expect("required");
                let block_step = args.get_one::<u64>("block-step").expect("required");
                let http_url = args.get_one::<String>("http-url").expect("required");
                println!("reconstruct from L1, starting from block number {}, processing {} blocks at a time", start_block, block_step);

                // TODO: This should be an env variable / CLI argument.
                let db_dir = env::current_dir()?.join("db");

                let fetcher = L1Fetcher::new(http_url)?;
                let processor = TreeProcessor::new(&db_dir)?;
                let (tx, rx) = mpsc::channel::<Vec<CommitBlockInfoV1>>(5);
                processor.run(rx);

                fetcher.fetch(tx, Some(U64([*start_block])), None).await?;
            }
            Some(("file", args)) => {
                let input_file = args.get_one::<String>("FILE").expect("required");
                println!("reconstruct from file (path: \"{}\")", input_file);
            }
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }

    Ok(())
}

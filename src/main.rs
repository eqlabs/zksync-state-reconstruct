use clap::{arg, value_parser, Command};

fn cli() -> Command {
    Command::new("state-reconstruct")
        .about("zkSync state reconstruction tool")
        .subcommand_required(true)
        .arg_required_else_help(false)
        .subcommand(
            Command::new("import")
                .about("Import state")
                .subcommand_required(true)
                .subcommand(
                    Command::new("l1")
                        .about("Import state from Ethereum L1")
                        .arg(
                            arg!(--"start-block" <START_BLOCK>)
                                .help("Ethereum block number to start state import from")
                                .default_value("16627460")
                                .value_parser(value_parser!(u64)),
                        )
                        .arg(
                            arg!(--"block-step" <BLOCK_STEP>)
                                .help("Number of blocks to filter & process in one step")
                                .default_value("128")
                                .value_parser(value_parser!(u64)),
                        ),
                ),
        )
}

fn main() {
    let matches = cli().get_matches();

    match matches.subcommand() {
        Some(("import", sub_matches)) => {
            match sub_matches.subcommand() {
                Some(("l1", args)) => {
                    let start_block = args.get_one::<u64>("start-block").expect("required");
                    let block_step = args.get_one::<u64>("block-step").expect("required");
                    println!("import from L1, starting from block number {}, processing {} blocks at a time", start_block, block_step);
                    // TODO(tuommaki): Implement block fetch logic.
                }
                _ => unreachable!(),
            }
        }
        _ => unreachable!(),
    }
}

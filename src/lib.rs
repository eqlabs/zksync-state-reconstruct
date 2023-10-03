#![feature(array_chunks)]
// #![warn(clippy::pedantic)]

mod snapshot;
mod tree;
mod types;

use crate::types::CommitBlockInfoV1;

use ethers::{
    abi::{Contract, Function},
    prelude::*,
    providers::Provider,
};
use eyre::Result;

pub const INITAL_STATE_PATH: &str = "InitialState.csv";
pub const STATE_FILE_PATH: &str = "StateSnapshot.json";
pub const ZK_SYNC_ADDR: &str = "0x32400084C286CF3E17e7B677ea9583e60a000324";
pub const GENESIS_BLOCK: u64 = 16_627_460;
pub const BLOCK_STEP: u64 = 128;

pub async fn init_eth_adapter(http_url: &str) -> (Provider<Http>, Contract) {
    let provider =
        Provider::<Http>::try_from(http_url).expect("could not instantiate HTTP Provider");

    let abi_file = std::fs::File::open("./IZkSync.json").unwrap();
    let contract = Contract::load(abi_file).unwrap();

    (provider, contract)
}

pub fn parse_calldata(
    commit_blocks_fn: &Function,
    calldata: &[u8],
) -> Result<Vec<CommitBlockInfoV1>> {
    let mut parsed_input = commit_blocks_fn
        .decode_input(&calldata[4..])
        .map_err(|e| types::ParseError::InvalidCalldata(e.to_string()))?;

    if parsed_input.len() != 2 {
        return Err(types::ParseError::InvalidCalldata(format!(
            "invalid number of parameters (got {}, expected 2) for commitBlocks function",
            parsed_input.len()
        ))
        .into());
    }

    let new_blocks_data = parsed_input
        .pop()
        .ok_or_else(|| types::ParseError::InvalidCalldata("new blocks data".to_string()))?;
    let stored_block_info = parsed_input
        .pop()
        .ok_or_else(|| types::ParseError::InvalidCalldata("stored block info".to_string()))?;

    let abi::Token::Tuple(stored_block_info) = stored_block_info else {
        return Err(
            types::ParseError::InvalidCalldata("invalid StoredBlockInfo".to_string()).into(),
        );
    };

    let abi::Token::Uint(_previous_l2_block_number) = stored_block_info[0].clone() else {
        return Err(types::ParseError::InvalidStoredBlockInfo(
            "cannot parse previous L2 block number".to_string(),
        )
        .into());
    };

    let abi::Token::Uint(_previous_enumeration_index) = stored_block_info[2].clone() else {
        return Err(types::ParseError::InvalidStoredBlockInfo(
            "cannot parse previous enumeration index".to_string(),
        )
        .into());
    };

    //let previous_enumeration_index = previous_enumeration_index.0[0];
    // TODO: What to do here?
    // assert_eq!(previous_enumeration_index, tree.next_enumeration_index());

    parse_commit_block_info(&new_blocks_data)
}

fn parse_commit_block_info(data: &abi::Token) -> Result<Vec<CommitBlockInfoV1>> {
    let mut res = vec![];

    let abi::Token::Array(data) = data else {
        return Err(types::ParseError::InvalidCommitBlockInfo(
            "cannot convert newBlocksData to array".to_string(),
        )
        .into());
    };

    for data in data.iter() {
        match CommitBlockInfoV1::try_from(data) {
            Ok(blk) => res.push(blk),
            Err(e) => println!("failed to parse commit block info: {}", e),
        }
    }

    Ok(res)
}

#[cfg(test)]
mod tests {
    use std::env;

    use ethers::{
        providers::Middleware,
        types::{Address, BlockNumber, Filter},
    };

    use eyre::Result;

    use crate::{snapshot::StateSnapshot, tree::TreeWrapper};

    use super::*;

    #[ignore]
    #[tokio::test]
    async fn it_works() -> Result<()> {
        // TODO: This should be an env variable / CLI argument.
        let db_dir = env::current_dir()?.join("db");

        // TODO: Implement graceful shutdown.
        // If database directory already exists, we try to restore the latest state.
        // The state contains the last processed block and a mapping of index to key
        // values, if a state file does not exist, we simply use the defaults instead.
        let should_restore_state = db_dir.exists();
        let mut state_snapshot = if should_restore_state {
            println!("Loading previous state file...");
            StateSnapshot::read(STATE_FILE_PATH).expect("state file is malformed")
        } else {
            println!("No existing database found, starting from genesis...");
            StateSnapshot::default()
        };

        // Extract fields from state snapshot.
        let StateSnapshot {
            mut current_l1_block_number,
            mut latest_l2_block_number,
            ref index_to_key_map,
        } = state_snapshot;

        let mut tree = TreeWrapper::new(db_dir.as_path(), index_to_key_map.clone())?;

        let (provider, contract) = init_eth_adapter("https://eth.llamarpc.com").await;
        let latest_l1_block_number = provider
            .get_block(BlockNumber::Latest)
            .await?
            .unwrap()
            .number
            .unwrap();

        let event = contract.events_by_name("BlockCommit")?[0].clone();
        let function = contract.functions_by_name("commitBlocks")?[0].clone();

        println!("Starting from l1 block: {}", current_l1_block_number);
        while current_l1_block_number <= latest_l1_block_number.0[0] {
            // Create a filter showing only `BlockCommit`s from the [`ZK_SYNC_ADDR`].
            // TODO: Filter by executed blocks too.
            let filter = Filter::new()
                .address(ZK_SYNC_ADDR.parse::<Address>()?)
                .topic0(event.signature())
                .from_block(current_l1_block_number)
                .to_block(current_l1_block_number + BLOCK_STEP);

            // Grab all relevant logs.
            let logs = provider.get_logs(&filter).await?;
            for log in logs {
                // log.topics:
                // topics[1]: L2 block number.
                // topics[2]: L2 block hash.
                // topics[3]: L2 commitment.

                let new_l2_block_number = U256::from_big_endian(log.topics[1].as_fixed_bytes());
                if new_l2_block_number <= latest_l2_block_number {
                    continue;
                }

                if let Some(tx_hash) = log.transaction_hash {
                    let tx = provider.get_transaction(tx_hash).await?.unwrap();
                    let calldata = tx.input;
                    let blocks = parse_calldata(&function, &calldata)?;

                    let num_blocks = blocks.len();
                    println!("Parsed {} new blocks", num_blocks);

                    for block in blocks {
                        latest_l2_block_number = tree.insert_block(&block);

                        // Update snapshot values.
                        state_snapshot.latest_l2_block_number = latest_l2_block_number;
                        state_snapshot.index_to_key_map = tree.index_to_key.clone();
                    }
                }
            }
            // Increment current block index.
            current_l1_block_number += BLOCK_STEP;

            // Update snapshot values and write the current state to a file.
            state_snapshot.current_l1_block_number = current_l1_block_number;
            state_snapshot.write(STATE_FILE_PATH)?;
        }

        Ok(())
    }
}

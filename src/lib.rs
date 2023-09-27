#![feature(array_chunks)]

use std::collections::HashMap;
use std::fs;
use thiserror::Error;

mod state;
use crate::state::CommitBlockInfoV1;

use ethers::{
    abi::{Contract, Function},
    prelude::*,
    providers::{Http, Provider},
};
use eyre::Result;

pub const INITAL_STATE_PATH: &str = "InitialState.csv";
pub const ZK_SYNC_ADDR: &str = "0x32400084C286CF3E17e7B677ea9583e60a000324";
pub const GENESIS_BLOCK: u64 = 16_627_460;
pub const BLOCK_STEP: u64 = 128;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("invalid Calldata: {0}")]
    InvalidCalldata(String),

    #[error("invalid StoredBlockInfo: {0}")]
    InvalidStoredBlockInfo(String),

    #[error("invalid CommitBlockInfo: {0}")]
    InvalidCommitBlockInfo(String),

    #[error("invalid compressed bytecode: {0}")]
    InvalidCompressedByteCode(String),
}

pub fn create_initial_state() {
    let _input = fs::read_to_string(INITAL_STATE_PATH).unwrap();
    todo!();
}

pub async fn init_eth_adapter(http_url: &str) -> (Provider<Http>, Contract) {
    let provider =
        Provider::<Http>::try_from(http_url).expect("could not instantiate HTTP Provider");

    let abi_file = std::fs::File::open("./IZkSync.json").unwrap();
    let contract = Contract::load(abi_file).unwrap();

    (provider, contract)
}

fn parse_calldata(
    l2_block_number: U256,
    commit_blocks_fn: &Function,
    calldata: &[u8],
) -> Result<Vec<CommitBlockInfoV1>> {
    let mut parsed_input = commit_blocks_fn
        .decode_input(&calldata[4..])
        .map_err(|e| ParseError::InvalidCalldata(e.to_string()))?;

    if parsed_input.len() != 2 {
        return Err(ParseError::InvalidCalldata(format!(
            "invalid number of parameters (got {}, expected 2) for commitBlocks function",
            parsed_input.len()
        ))
        .into());
    }

    let new_blocks_data = parsed_input
        .pop()
        .ok_or_else(|| ParseError::InvalidCalldata("new blocks data".to_string()))?;
    let stored_block_info = parsed_input
        .pop()
        .ok_or_else(|| ParseError::InvalidCalldata("stored block info".to_string()))?;

    let abi::Token::Tuple(stored_block_info) = stored_block_info else {
        return Err(ParseError::InvalidCalldata("invalid StoredBlockInfo".to_string()).into());
    };

    let abi::Token::Uint(previous_l2_block_number) = stored_block_info[0].clone() else {
        return Err(ParseError::InvalidStoredBlockInfo(
            "cannot parse previous L2 block number".to_string(),
        )
        .into());
    };

    let abi::Token::Uint(_previous_enumeration_index) = stored_block_info[2].clone() else {
        return Err(ParseError::InvalidStoredBlockInfo(
            "cannot parse previous enumeration index".to_string(),
        )
        .into());
    };

    //let previous_enumeration_index = previous_enumeration_index.0[0];
    // TODO: What to do here?
    // assert_eq!(previous_enumeration_index, tree.next_enumeration_index());

    parse_commit_block_info(l2_block_number, &new_blocks_data)
}

fn parse_commit_block_info(
    l2_block_number: U256,
    data: &abi::Token,
) -> Result<Vec<CommitBlockInfoV1>> {
    let mut res = vec![];
    let mut latest_l2_block_number = l2_block_number;

    let abi::Token::Array(data) = data else {
        return Err(ParseError::InvalidCommitBlockInfo(
            "cannot convert newBlocksData to array".to_string(),
        )
        .into());
    };

    for data in data.iter() {
        let abi::Token::Tuple(block_elems) = data else {
            return Err(ParseError::InvalidCommitBlockInfo("struct elements".to_string()).into());
        };
        let abi::Token::Uint(new_l2_block_number) = block_elems[0].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("blockNumber".to_string()).into());
        };

        /* TODO(tuommaki): Fix the check below.
        if new_l2_block_number <= latest_l2_block_number {
            println!("skipping before we even get started");
            continue;
        }
        */

        let abi::Token::Uint(timestamp) = block_elems[1].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("timestamp".to_string()).into());
        };

        let abi::Token::Uint(new_enumeration_index) = block_elems[2].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "indexRepeatedStorageChanges".to_string(),
            )
            .into());
        };
        let new_enumeration_index = new_enumeration_index.0[0];

        let abi::Token::FixedBytes(state_root) = block_elems[3].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("newStateRoot".to_string()).into());
        };

        let abi::Token::Uint(number_of_l1_txs) = block_elems[4].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("numberOfLayer1Txs".to_string()).into());
        };

        let abi::Token::FixedBytes(l2_logs_tree_root) = block_elems[5].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("l2LogsTreeRoot".to_string()).into());
        };

        let abi::Token::FixedBytes(priority_operations_hash) = block_elems[6].clone() else {
            return Err(
                ParseError::InvalidCommitBlockInfo("priorityOperationsHash".to_string()).into(),
            );
        };

        let abi::Token::Bytes(initial_changes_calldata) = block_elems[7].clone() else {
            return Err(
                ParseError::InvalidCommitBlockInfo("initialStorageChanges".to_string()).into(),
            );
        };

        if initial_changes_calldata.len() % 64 != 4 {
            return Err(
                ParseError::InvalidCommitBlockInfo("initialStorageChanges".to_string()).into(),
            );
        }
        let abi::Token::Bytes(repeated_changes_calldata) = block_elems[8].clone() else {
            return Err(
                ParseError::InvalidCommitBlockInfo("repeatedStorageChanges".to_string()).into(),
            );
        };

        let abi::Token::Bytes(l2_logs) = block_elems[9].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("l2Logs".to_string()).into());
        };

        // TODO(tuommaki): Are these useful at all?
        /*
        let abi::Token::Bytes(_l2_arbitrary_length_msgs) = block_elems[10].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "l2ArbitraryLengthMessages".to_string(),
            )
            .into());
        };
        */

        // TODO(tuommaki): Parse factory deps
        let abi::Token::Array(factory_deps) = block_elems[11].clone() else {
            return Err(ParseError::InvalidCommitBlockInfo("factoryDeps".to_string()).into());
        };

        let mut smartcontracts = vec![];
        for bytecode in factory_deps.into_iter() {
            let abi::Token::Bytes(bytecode) = bytecode else {
                return Err(ParseError::InvalidCommitBlockInfo("factoryDeps".to_string()).into());
            };

            match decompress_bytecode(bytecode) {
                Ok(bytecode) => smartcontracts.push(bytecode),
                Err(e) => println!("failed to decompress bytecode: {}", e),
            };
        }

        assert_eq!(repeated_changes_calldata.len() % 40, 4);

        println!(
            "Have {} new keys",
            (initial_changes_calldata.len() - 4) / 64
        );
        println!(
            "Have {} repeated keys",
            (repeated_changes_calldata.len() - 4) / 40
        );

        let mut blk = CommitBlockInfoV1 {
            block_number: new_l2_block_number.as_u64(),
            timestamp: timestamp.as_u64(),
            index_repeated_storage_changes: new_enumeration_index,
            new_state_root: state_root,
            number_of_l1_txs,
            l2_logs_tree_root,
            priority_operations_hash,
            initial_storage_changes: HashMap::default(),
            repeated_storage_changes: HashMap::default(),
            l2_logs: l2_logs.to_vec(),
            factory_deps: smartcontracts,
        };

        for initial_calldata in initial_changes_calldata[4..].chunks(64) {
            let mut t = initial_calldata.array_chunks::<32>();
            let key = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("initialStorageChanges".to_string())
            })?;
            let value = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("initialStorageChanges".to_string())
            })?;

            if t.next().is_some() {
                return Err(ParseError::InvalidCommitBlockInfo(
                    "initialStorageChanges".to_string(),
                )
                .into());
            }

            let _ = blk.initial_storage_changes.insert(key, value);
        }

        for repeated_calldata in repeated_changes_calldata[4..].chunks(40) {
            let index = u64::from_be_bytes([
                repeated_calldata[0],
                repeated_calldata[1],
                repeated_calldata[2],
                repeated_calldata[3],
                repeated_calldata[4],
                repeated_calldata[5],
                repeated_calldata[6],
                repeated_calldata[7],
            ]);
            let mut t = repeated_calldata[8..].array_chunks::<32>();
            let value = *t.next().ok_or_else(|| {
                ParseError::InvalidCommitBlockInfo("repeatedStorageChanges".to_string())
            })?;

            if t.next().is_some() {
                return Err(ParseError::InvalidCommitBlockInfo(
                    "repeatedStorageChanges".to_string(),
                )
                .into());
            }

            blk.repeated_storage_changes.insert(index, value);
        }

        // --------------------------------------------------------------
        // TODO(tuommaki): Handle rest of the fields:
        //                     - factoryDeps -> compressed bytecode
        // --------------------------------------------------------------

        latest_l2_block_number = new_l2_block_number;
        res.push(blk);
    }

    Ok(res)
}

fn decompress_bytecode(data: Vec<u8>) -> Result<Vec<u8>> {
    /*
    let num_entries = u32::from_be_bytes([data[3], data[2], data[1], data[0]]);

    */
    let mut offset = 0;

    let dict_len = u16::from_be_bytes([data[offset + 1], data[offset]]);

    offset += 2;

    let end = 2 + dict_len as usize;
    let dict = data[offset..end].to_vec();
    offset += end;
    let encoded_data = data[offset..].to_vec();

    // Each dictionary element should be 8 bytes. Verify alignment.
    if dict.len() % 8 != 0 {
        return Err(ParseError::InvalidCompressedByteCode(format!(
            "invalid dict length: {}",
            dict.len()
        ))
        .into());
    }

    let dict: Vec<&[u8]> = dict.chunks(8).collect();

    // Verify that dictionary size is below maximum.
    if dict.len() > (1 << 16)
    /* 2^16 */
    {
        return Err(ParseError::InvalidCompressedByteCode(format!(
            "too many elements in dictionary: {}",
            dict.len()
        ))
        .into());
    }

    let mut bytecode = vec![];
    for idx in encoded_data.chunks(2) {
        let idx = u16::from_be_bytes([idx[0], idx[1]]) as usize;
        if dict.len() <= idx {
            return Err(ParseError::InvalidCompressedByteCode(format!(
                "encoded data index ({}) exceeds dictionary size ({})",
                idx,
                dict.len()
            ))
            .into());
        }

        bytecode.append(&mut dict[idx].to_vec());
    }

    Ok(bytecode)
}

#[cfg(test)]
mod tests {
    use ethers::{
        providers::Middleware,
        types::{Address, BlockNumber, Filter},
    };

    use eyre::Result;

    use super::*;

    #[tokio::test]
    async fn it_works() -> Result<()> {
        let (provider, contract) = init_eth_adapter().await;
        let latest_block = provider
            .get_block(BlockNumber::Latest)
            .await?
            .unwrap()
            .number
            .unwrap();

        let event = contract.events_by_name("BlockCommit")?[0].clone();
        let function = contract.functions_by_name("commitBlocks")?[0].clone();

        let mut current_block = GENESIS_BLOCK;
        let mut latest_l2_block_number = U256::default();
        while current_block <= latest_block.0[0] {
            // Create a filter showing only `BlockCommit`s from the [`ZK_SYNC_ADDR`].
            // TODO: Filter by executed blocks too.
            let filter = Filter::new()
                .address(ZK_SYNC_ADDR.parse::<Address>()?)
                .topic0(event.signature())
                .from_block(current_block)
                .to_block(current_block + BLOCK_STEP);

            // Grab all relevant logs.
            let logs = provider.get_logs(&filter).await?;

            println!("{}", logs.iter().len());
            for log in logs {
                println!("{:?}", log);
                // log.topics:
                // topics[1]: L2 block number.
                // topics[2]: L2 block hash.
                // topics[3]: L2 commitment.
                // TODO: Check for already processed blocks.

                let new_l2_block_number = U256::from_big_endian(log.topics[1].as_fixed_bytes());
                if new_l2_block_number <= latest_l2_block_number {
                    continue;
                }

                if let Some(tx_hash) = log.transaction_hash {
                    let tx = provider.get_transaction(tx_hash).await?;
                    let calldata = tx.unwrap().input.to_vec();
                    let blocks = parse_calldata(new_l2_block_number, &function, calldata)?;

                    // TODO: Apply transaction to L2.
                    latest_l2_block_number = new_l2_block_number;

                    println!("parsed {} new blocks", blocks.len());
                }
            }

            // Increment current block index.
            current_block += BLOCK_STEP;
        }

        Ok(())
    }
}

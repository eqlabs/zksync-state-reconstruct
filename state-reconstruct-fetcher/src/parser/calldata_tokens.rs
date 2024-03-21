use ethers::{
    abi::{self, Token},
    types::U256,
};

use crate::ParseError;

pub struct CalldataToken {
    pub new_blocks_data: NewBlocksDataToken,
    pub stored_block_info: StoredBlockInfoToken,
}

impl TryFrom<Vec<Token>> for CalldataToken {
    type Error = ParseError;

    fn try_from(mut value: Vec<Token>) -> Result<Self, Self::Error> {
        if value.len() != 2 {
            return Err(ParseError::InvalidCalldata(format!(
                "invalid number of parameters (got {}, expected 2) for commitBlocks function",
                value.len()
            )));
        }

        let new_blocks_data = value
            .pop()
            .ok_or_else(|| ParseError::InvalidCalldata("new blocks data".to_string()))?;
        let stored_block_info = value
            .pop()
            .ok_or_else(|| ParseError::InvalidCalldata("stored block info".to_string()))?;

        Ok(CalldataToken {
            new_blocks_data: NewBlocksDataToken::try_from(new_blocks_data)?,
            stored_block_info: StoredBlockInfoToken::try_from(stored_block_info)?,
        })
    }
}

pub struct NewBlocksDataToken {
    pub data: Vec<Token>,
}

impl TryFrom<Token> for NewBlocksDataToken {
    type Error = ParseError;

    fn try_from(value: Token) -> Result<Self, Self::Error> {
        let abi::Token::Array(data) = value else {
            return Err(ParseError::InvalidCommitBlockInfo(
                "cannot convert newBlocksData to array".to_string(),
            ));
        };

        Ok(Self { data })
    }
}

pub struct StoredBlockInfoToken {
    pub previous_l2_block_number: U256,
    pub previous_enumeration_index: U256,
}

impl TryFrom<Token> for StoredBlockInfoToken {
    type Error = ParseError;

    fn try_from(value: Token) -> Result<Self, Self::Error> {
        let abi::Token::Tuple(stored_block_info) = value else {
            return Err(ParseError::InvalidCalldata(
                "invalid StoredBlockInfo".to_string(),
            ));
        };

        let abi::Token::Uint(previous_l2_block_number) = stored_block_info[0].clone() else {
            return Err(ParseError::InvalidStoredBlockInfo(
                "cannot parse previous L2 block number".to_string(),
            ));
        };

        let abi::Token::Uint(previous_enumeration_index) = stored_block_info[2].clone() else {
            return Err(ParseError::InvalidStoredBlockInfo(
                "cannot parse previous enumeration index".to_string(),
            ));
        };

        Ok(StoredBlockInfoToken {
            previous_l2_block_number,
            previous_enumeration_index,
        })
    }
}

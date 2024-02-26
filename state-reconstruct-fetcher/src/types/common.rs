//! A collection of functions that get reused throughout format versions.
// TODO: Move these to a dedicated parser struct.
use ethers::types::U256;

use super::{PackingType, ParseError};
use crate::constants::zksync::{LENGTH_BITS_OFFSET, OPERATION_BITMASK};

pub fn read_compressed_value(bytes: &[u8], pointer: &mut usize) -> Result<PackingType, ParseError> {
    let metadata = u8::from_be_bytes(read_next_n_bytes(bytes, pointer));
    let operation = metadata & OPERATION_BITMASK;
    let len = if operation == 0 {
        32
    } else {
        metadata >> LENGTH_BITS_OFFSET
    } as usize;

    // Read compressed value.
    let mut buffer = [0; 32];
    let start = buffer.len() - len;
    buffer[start..].copy_from_slice(&bytes[*pointer..*pointer + len]);
    *pointer += len;
    let compressed_value = U256::from_big_endian(&buffer);

    let packing_type = match operation {
        0 => PackingType::NoCompression(compressed_value),
        1 => PackingType::Add(compressed_value),
        2 => PackingType::Sub(compressed_value),
        3 => PackingType::Transform(compressed_value),
        _ => {
            return Err(ParseError::InvalidCompressedValue(String::from(
                "UnknownPackingType",
            )))
        }
    };

    Ok(packing_type)
}

pub fn read_next_n_bytes<const N: usize>(bytes: &[u8], pointer: &mut usize) -> [u8; N] {
    if *pointer >= bytes.len() {
        return [0; N];
    }
    let mut buffer = [0; N];
    buffer.copy_from_slice(&bytes[*pointer..*pointer + N]);
    *pointer += N;
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PackingType;

    #[test]
    fn parse_compressed_value_common() {
        let buf = [
            0, 1, 0, 0, 197, 168, 90, 55, 47, 68, 26, 198, 147, 33, 10, 24, 230, 131, 181, 48, 190,
            216, 117, 253, 202, 178, 247, 225, 1, 176, 87, 212, 51,
        ];
        let mut pointer = 0;
        let packing_type = read_compressed_value(&buf, &mut pointer).unwrap();
        let PackingType::NoCompression(_n) = packing_type else {
            panic!("packing_type = {:?}", packing_type);
        };
    }

    #[test]
    fn parse_compressed_value_add() {
        let buf = [9, 1];
        let mut pointer = 0;
        let packing_type = read_compressed_value(&buf, &mut pointer).unwrap();
        if let PackingType::Add(n) = packing_type {
            assert_eq!(n, U256::one());
        } else {
            panic!("packing_type = {:?}", packing_type);
        }
    }
}

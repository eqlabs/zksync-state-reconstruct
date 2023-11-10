use ethers::types::H256;

// These are copied from zkSync-era project to avoid pulling in full dependency.
pub fn bytes_to_chunks(bytes: &[u8]) -> Vec<[u8; 32]> {
    assert!(
        bytes.len() % 32 == 0,
        "Bytes must be divisible by 32 to split into chunks"
    );

    bytes
        .chunks(32)
        .map(|el| {
            let mut chunk = [0u8; 32];
            chunk.copy_from_slice(el);
            chunk
        })
        .collect()
}

pub fn hash_bytecode(code: &[u8]) -> H256 {
    let chunked_code = bytes_to_chunks(code);
    let hash =
        zkevm_opcode_defs::utils::bytecode_to_code_hash(&chunked_code).expect("Invalid bytecode");

    H256(hash)
}

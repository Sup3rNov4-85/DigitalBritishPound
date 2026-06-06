use crate::{
    consensus::{block_subsidy_units, GENESIS_DIFFICULTY_TARGET},
    crypto::{
        british_work::pow_hash,
        wallet::Address,
    },
    hash_meets_target,
    compute_merkle_root, Block, BlockHeader, Hash, Script, Transaction, TxInput, TxOutput,
};

/// Whitepaper genesis coinbase message (UTF-8).
pub const GENESIS_MESSAGE: &str =
    "The Times 27/May/2026 — A nation overtaxed, searching for an alternative";

/// Build an unmined genesis block template (nonce = 0).
pub fn genesis_template(timestamp: u32, payout: Address) -> Result<Block, String> {
    let msg = GENESIS_MESSAGE.as_bytes();
    let coinbase_input = TxInput::new_coinbase(msg.to_vec()).map_err(|e| e.to_string())?;
    let mut spk = Vec::with_capacity(21);
    spk.push(0x14);
    spk.extend_from_slice(payout.as_bytes());
    let subsidy = block_subsidy_units(0);
    let coinbase = Transaction {
        version: 1,
        inputs: vec![coinbase_input],
        outputs: vec![TxOutput::new(subsidy, Script::new(spk))],
        locktime: 0,
    };
    let txs = vec![coinbase];
    let merkle_root = compute_merkle_root(&txs).map_err(|e| e.to_string())?;
    let header = BlockHeader {
        version: 1,
        previous_block_hash: Hash::ZERO,
        merkle_root,
        timestamp,
        difficulty_target: GENESIS_DIFFICULTY_TARGET,
        nonce: 0,
        height: 0,
        uncle_hashes: vec![],
    };
    Ok(Block {
        header,
        transactions: txs,
        uncle_blocks: vec![],
    })
}

/// Mine genesis (grind nonce) with BritishWork.
pub fn mine_genesis(timestamp: u32, payout: Address) -> Result<Block, String> {
    let mut block = genesis_template(timestamp, payout)?;
    loop {
        let pow = pow_hash(&block.header)?;
        if hash_meets_target(&pow, block.header.difficulty_target) {
            break;
        }
        block.header.nonce = block.header.nonce.wrapping_add(1);
    }
    Ok(block)
}

/// Convenience: mine genesis to a throwaway wallet address.
pub fn mine_genesis_default(timestamp: u32) -> Result<Block, String> {
    let m = crate::crypto::wallet::Wallet::generate_mnemonic();
    let w = crate::crypto::wallet::Wallet::from_mnemonic(&m).map_err(|e| e.to_string())?;
    mine_genesis(timestamp, w.address())
}

/// Load a published genesis block from JSON (same format as `export-genesis`).
pub fn import_genesis_json(path: &std::path::Path) -> Result<Block, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

/// Write genesis block JSON and print its hash (for forum posts — no IP required).
pub fn export_genesis_json(block: &Block, path: &std::path::Path) -> Result<(), String> {
    let hash = block.compute_hash().map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(block).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    println!("genesis hash={}", hash.to_hex());
    println!("written to {}", path.display());
    println!("publish ONLY the hash + this file — not your peer id or IP");
    Ok(())
}

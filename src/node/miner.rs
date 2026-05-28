use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    consensus::block_subsidy_units,
    crypto::{british_work::pow_hash, wallet::Address},
    hash_meets_target,
    node::uncles::uncle_reward_units,
    compute_merkle_root, Block, BlockHeader, Hash, Script, Transaction, TxInput, TxOutput,
};

#[derive(Debug, thiserror::Error)]
pub enum MinerError {
    #[error("serialization error: {0}")]
    Ser(String),
    #[error("invalid merkle root")]
    BadMerkle,
}

pub struct Miner;

impl Miner {
    pub fn mine_next_block(
        prev_hash: Hash,
        height: u64,
        difficulty_target: u32,
        payout: Address,
        coinbase_message: &[u8],
        transactions: Vec<Transaction>,
        fees_units: u64,
        uncle_blocks: Vec<BlockHeader>,
    ) -> Result<Block, MinerError> {
        let mut cb_data = Vec::new();
        cb_data.extend_from_slice(&height.to_le_bytes());
        cb_data.extend_from_slice(coinbase_message);
        let coinbase_input = TxInput::new_coinbase(cb_data).map_err(|e| MinerError::Ser(e.to_string()))?;

        let mut spk = Vec::with_capacity(1 + 20);
        spk.push(0x14);
        spk.extend_from_slice(payout.as_bytes());
        let script_pubkey = Script::new(spk);

        let mut uncle_hashes = Vec::new();
        for u in &uncle_blocks {
            let bytes = bincode::serialize(u).map_err(|e| MinerError::Ser(e.to_string()))?;
            uncle_hashes.push(Hash::from_bytes(*blake3::hash(&bytes).as_bytes()));
        }

        let subsidy = block_subsidy_units(height).saturating_add(fees_units);
        let coinbase = Transaction {
            version: 1,
            inputs: vec![coinbase_input],
            outputs: vec![TxOutput::new(subsidy, script_pubkey)],
            locktime: 0,
        };

        let mut txs = Vec::with_capacity(1 + transactions.len());
        txs.push(coinbase);
        txs.extend(transactions);
        let merkle_root = compute_merkle_root(&txs).map_err(|e| MinerError::Ser(e.to_string()))?;

        let timestamp = now_secs();
        let header = BlockHeader {
            version: 1,
            previous_block_hash: prev_hash,
            merkle_root,
            timestamp,
            difficulty_target,
            nonce: 0,
            height,
            uncle_hashes,
        };

        let block = Block {
            header,
            transactions: txs,
            uncle_blocks,
        };

        // Adjust coinbase for uncle rewards after we know uncles.
        let mut block = block;
        let uncle_pay = uncle_reward_units(&block).map_err(|e| MinerError::Ser(e.to_string()))?;
        if uncle_pay > 0 {
            block.transactions[0].outputs[0].value = block.transactions[0].outputs[0]
                .value
                .saturating_add(uncle_pay);
            let merkle_root =
                compute_merkle_root(&block.transactions).map_err(|e| MinerError::Ser(e.to_string()))?;
            block.header.merkle_root = merkle_root;
        }

        // BritishWork PoW grind on header.
        loop {
            let pow = pow_hash(&block.header).map_err(|e| MinerError::Ser(e))?;
            if hash_meets_target(&pow, block.header.difficulty_target) {
                break;
            }
            block.header.nonce = block.header.nonce.wrapping_add(1);
            if block.header.nonce == 0 {
                block.header.timestamp = now_secs();
            }
        }

        if !block.verify_merkle_root().map_err(|e| MinerError::Ser(e.to_string()))? {
            return Err(MinerError::BadMerkle);
        }
        Ok(block)
    }
}

fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

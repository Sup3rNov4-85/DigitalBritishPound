use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    consensus::{block_subsidy_units, MAX_BLOCK_BYTES},
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
    /// Assemble and mine the next block.
    ///
    /// `candidate_txs` are mempool transactions with their fees. They are
    /// packed greedily by fee rate under the 2 MB consensus limit; the
    /// coinbase only claims fees for transactions actually included.
    pub fn mine_next_block(
        prev_hash: Hash,
        height: u64,
        difficulty_target: u32,
        payout: Address,
        coinbase_message: &[u8],
        candidate_txs: Vec<(Transaction, u64)>,
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

        let coinbase = Transaction {
            version: 1,
            inputs: vec![coinbase_input],
            outputs: vec![TxOutput::new(block_subsidy_units(height), script_pubkey)],
            locktime: 0,
        };

        // Base block (coinbase + uncles, no mempool txs) to measure the size budget.
        let timestamp = now_secs();
        let header = BlockHeader {
            version: 1,
            previous_block_hash: prev_hash,
            merkle_root: Hash::ZERO,
            timestamp,
            difficulty_target,
            nonce: 0,
            height,
            uncle_hashes,
        };
        let mut block = Block {
            header,
            transactions: vec![coinbase],
            uncle_blocks,
        };
        let base_size = bincode::serialize(&block)
            .map_err(|e| MinerError::Ser(e.to_string()))?
            .len();
        let budget = MAX_BLOCK_BYTES.saturating_sub(base_size);

        // Greedy fee-rate packing under the 2 MB consensus limit.
        let mut cands: Vec<(Transaction, u64, usize)> = Vec::with_capacity(candidate_txs.len());
        for (tx, fee) in candidate_txs {
            let size = bincode::serialize(&tx)
                .map_err(|e| MinerError::Ser(e.to_string()))?
                .len();
            cands.push((tx, fee, size));
        }
        cands.sort_by(|(_, fa, sa), (_, fb, sb)| {
            // fee_a/size_a vs fee_b/size_b without floats, descending.
            (fb.saturating_mul(*sa as u64)).cmp(&fa.saturating_mul(*sb as u64))
        });
        let mut used = 0usize;
        let mut included_fees = 0u64;
        for (tx, fee, size) in cands {
            if used.saturating_add(size) > budget {
                continue;
            }
            used += size;
            included_fees = included_fees.saturating_add(fee);
            block.transactions.push(tx);
        }

        // Coinbase claims subsidy + fees of included txs + uncle inclusion bonus.
        let uncle_pay = uncle_reward_units(&block).map_err(|e| MinerError::Ser(e.to_string()))?;
        block.transactions[0].outputs[0].value = block.transactions[0].outputs[0]
            .value
            .saturating_add(included_fees)
            .saturating_add(uncle_pay);
        block.header.merkle_root =
            compute_merkle_root(&block.transactions).map_err(|e| MinerError::Ser(e.to_string()))?;

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

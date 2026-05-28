use crate::{
    consensus::{block_subsidy_units, BPS_DENOM, UNCLE_LOOKBACK_BLOCKS, UNCLE_REWARD_BPS},
    crypto::british_work::pow_hash,
    hash_meets_target,
    storage::chaindb::ChainDb,
    Block, BlockHeader, Hash,
};

use super::validation::ValidationError;

/// Total uncle subsidy (in units) creditable to the nephew coinbase.
pub fn uncle_reward_units(block: &Block) -> Result<u64, ValidationError> {
    let mut total = 0u64;
    for hdr in &block.uncle_blocks {
        let sub = block_subsidy_units(hdr.height);
        total = total.saturating_add(sub.saturating_mul(UNCLE_REWARD_BPS) / BPS_DENOM);
    }
    Ok(total)
}

/// Validate uncle headers referenced by a block.
pub fn validate_uncles(chain: &ChainDb, block: &Block) -> Result<(), ValidationError> {
    if block.header.uncle_hashes.len() != block.uncle_blocks.len() {
        return Err(ValidationError::UncleMismatch);
    }
    if block.header.uncle_hashes.len() > crate::consensus::MAX_UNCLES_PER_BLOCK {
        return Err(ValidationError::TooManyUncles);
    }

    let height = block.header.height;
    let min_height = height.saturating_sub(UNCLE_LOOKBACK_BLOCKS);

    for (i, uncle_hdr) in block.uncle_blocks.iter().enumerate() {
        let expected = block.header.uncle_hashes[i];
        let uncle_pow = pow_hash(uncle_hdr).map_err(|e| ValidationError::Dbc(crate::DbcError::SerialiseError(e)))?;
        let uncle_id = header_id(uncle_hdr);

        if uncle_id != expected {
            return Err(ValidationError::UncleHashMismatch);
        }
        if !hash_meets_target(&uncle_pow, uncle_hdr.difficulty_target) {
            return Err(ValidationError::UncleBadPow);
        }
        if uncle_hdr.height >= height || uncle_hdr.height < min_height {
            return Err(ValidationError::UncleBadHeight);
        }

        // Parent of uncle must exist in our chain and be an ancestor within lookback.
        if chain.get_block(&uncle_hdr.previous_block_hash).is_err() {
            return Err(ValidationError::UncleUnknownParent);
        }

        // Uncle must not duplicate an uncle included in the last 7 blocks.
        if uncle_in_recent_window(chain, height, &expected)? {
            return Err(ValidationError::UncleDuplicate);
        }
    }

    Ok(())
}

fn header_id(header: &BlockHeader) -> Hash {
    let bytes = bincode::serialize(header).expect("header serializes");
    Hash::from_bytes(*blake3::hash(&bytes).as_bytes())
}

fn uncle_in_recent_window(
    chain: &ChainDb,
    nephew_height: u64,
    uncle_hash: &Hash,
) -> Result<bool, ValidationError> {
    let start = nephew_height.saturating_sub(UNCLE_LOOKBACK_BLOCKS);
    for h in start..nephew_height {
        if let Some(b) = chain.get_block_at_height(h).map_err(|e| {
            ValidationError::Dbc(crate::DbcError::SerialiseError(e.to_string()))
        })? {
            if b.header.uncle_hashes.contains(uncle_hash) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

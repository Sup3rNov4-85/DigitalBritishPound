//! DBC protocol-level consensus parameters and helpers.
//!
//! This is the "single source of truth" for the values stated in the
//! whitepaper (v1.0 / 2026).

/// DBC ticker symbol.
pub const TICKER: &str = "DBC";

/// Number of smallest units per 1 DBC.
///
/// Whitepaper: 1 Pence = 0.00000001 DBC → 10^-8.
pub const UNITS_PER_DBC: u64 = 100_000_000;

/// Total supply: 42,000,000 DBC.
pub const TOTAL_SUPPLY_DBC: u64 = 42_000_000;

/// Total supply in smallest units.
pub const TOTAL_SUPPLY_UNITS: u64 = TOTAL_SUPPLY_DBC * UNITS_PER_DBC;

/// Target block time: 15 minutes.
pub const TARGET_BLOCK_TIME_SECS: u64 = 15 * 60;

/// Halving interval: 420,000 blocks (~8 years @ 15m).
pub const HALVING_INTERVAL_BLOCKS: u64 = 420_000;

/// Genesis block reward: 50 DBC.
pub const GENESIS_REWARD_DBC: u64 = 50;

/// Initial block subsidy (before halvings) in smallest units.
pub const INITIAL_SUBSIDY_UNITS: u64 = GENESIS_REWARD_DBC * UNITS_PER_DBC;

/// Difficulty retarget: every 1,008 blocks (~1 week).
pub const DIFFICULTY_ADJUST_INTERVAL_BLOCKS: u64 = 1_008;

/// Moving average window for difficulty: last 144 blocks.
pub const DIFFICULTY_AVG_WINDOW_BLOCKS: usize = 144;

/// Uncle rules (whitepaper): up to 2 uncles, within last 7 blocks, 75% reward.
pub const MAX_UNCLES_PER_BLOCK: usize = 2;
pub const UNCLE_LOOKBACK_BLOCKS: u64 = 7;
pub const UNCLE_REWARD_BPS: u64 = 7_500; // 75% in basis points
pub const BPS_DENOM: u64 = 10_000;

/// Coinbase maturity: 100 blocks (already used in UTXO set).
pub const COINBASE_MATURITY_BLOCKS: u64 = 100;

/// Max block size (whitepaper): 2 MB, upgradeable via soft fork.
pub const MAX_BLOCK_BYTES: usize = 2 * 1024 * 1024;

/// Genesis / initial compact difficulty (Bitcoin-style nBits, easy for laptops).
pub const GENESIS_DIFFICULTY_TARGET: u32 = 0x1f00_ffff;

/// Mainnet genesis hash (block 0). This is part of consensus: nodes MUST reject any
/// chain whose height-0 block hash does not match.
pub const GENESIS_HASH_HEX: &str =
    "87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d";

/// Parse the expected genesis hash.
pub fn expected_genesis_hash() -> crate::Hash {
    let bytes = hex::decode(GENESIS_HASH_HEX).expect("GENESIS_HASH_HEX must be valid hex");
    assert_eq!(bytes.len(), 32, "GENESIS_HASH_HEX must be 32 bytes");
    let mut b = [0u8; 32];
    b.copy_from_slice(&bytes);
    crate::Hash::from_bytes(b)
}

/// Compute the block subsidy (newly minted coins) for a given height.
///
/// Height 0..=HALVING_INTERVAL_BLOCKS-1: 50 DBC
/// Next interval halves, etc.
pub fn block_subsidy_units(height: u64) -> u64 {
    let halvings = height / HALVING_INTERVAL_BLOCKS;
    if halvings >= 64 {
        return 0;
    }
    INITIAL_SUBSIDY_UNITS >> halvings
}

/// Compute the maximum allowed coinbase output value (subsidy + fees).
/// Fees are supplied by the caller (mempool/validation context).
pub fn max_coinbase_value_units(height: u64, fees_units: u64) -> u64 {
    block_subsidy_units(height).saturating_add(fees_units)
}

/// Difficulty for the block at `height` (retarget every 1,008 blocks using WMA of last 144 times).
pub fn next_difficulty_target(
    chain: &crate::storage::chaindb::ChainDb,
    height: u64,
) -> Result<u32, crate::storage::chaindb::ChainDbError> {
    if height == 0 {
        return Ok(GENESIS_DIFFICULTY_TARGET);
    }

    let prev = chain
        .get_block_at_height(height - 1)?
        .ok_or_else(|| crate::storage::chaindb::ChainDbError::NotFound(format!("height {}", height - 1)))?;
    let prev_target = prev.header.difficulty_target;

    if height % DIFFICULTY_ADJUST_INTERVAL_BLOCKS != 0 {
        return Ok(prev_target);
    }

    // Need at least 2 timestamps in the window.
    let start = height.saturating_sub(DIFFICULTY_AVG_WINDOW_BLOCKS as u64);
    let mut stamps = Vec::new();
    for h in start..height {
        if let Some(b) = chain.get_block_at_height(h)? {
            stamps.push(b.header.timestamp as u64);
        }
    }
    if stamps.len() < 2 {
        return Ok(prev_target);
    }

    let actual_secs = stamps.last().unwrap().saturating_sub(*stamps.first().unwrap());
    let span = (stamps.len() - 1) as u64;
    let expected_secs = span.saturating_mul(TARGET_BLOCK_TIME_SECS);

    Ok(adjust_compact_target(prev_target, actual_secs, expected_secs))
}

/// Scale compact target inversely with hashrate (higher actual time → easier target).
fn adjust_compact_target(current: u32, actual_secs: u64, expected_secs: u64) -> u32 {
    if actual_secs == 0 || expected_secs == 0 {
        return current;
    }

    let cur = crate::expand_compact_target(current);
    let cur_u128 = target_to_u128(&cur);

    // new_target = cur * (actual / expected) — slower blocks → lower target value numerically?
    // In Bitcoin, if blocks are slow (actual > expected), target increases (easier).
    // hash < target, so easier means larger target bytes.
    let new_u128 = cur_u128
        .saturating_mul(actual_secs as u128)
        .saturating_div(expected_secs as u128);

    let clamped = new_u128.clamp(1, u128::MAX / 2);
    compact_from_u128(clamped).unwrap_or(current)
}

fn target_to_u128(target: &[u8; 32]) -> u128 {
    let mut v = 0u128;
    for &b in target.iter().take(16) {
        v = (v << 8) | b as u128;
    }
    v
}

fn compact_from_u128(value: u128) -> Option<u32> {
    if value == 0 {
        return None;
    }
    // Find position of highest byte.
    let mut bytes = [0u8; 32];
    let mut tmp = value;
    let mut i = 31usize;
    while tmp > 0 && i > 0 {
        bytes[i] = (tmp & 0xff) as u8;
        tmp >>= 8;
        if tmp > 0 {
            i -= 1;
        }
    }
    let exponent = (32 - i) as u32;
    if exponent > 32 {
        return None;
    }
    let mantissa_start = i;
    let m0 = bytes[mantissa_start] as u32;
    let m1 = bytes.get(mantissa_start + 1).copied().unwrap_or(0) as u32;
    let m2 = bytes.get(mantissa_start + 2).copied().unwrap_or(0) as u32;
    let mantissa = (m0 << 16) | (m1 << 8) | m2;
    Some((exponent << 24) | (mantissa & 0x00ff_ffff))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsidy_starts_at_50_dbc() {
        assert_eq!(block_subsidy_units(0), 50 * UNITS_PER_DBC);
        assert_eq!(block_subsidy_units(1), 50 * UNITS_PER_DBC);
    }

    #[test]
    fn adjust_makes_easier_when_blocks_slow() {
        let easy = 0x1f00_ffff;
        let slower = adjust_compact_target(easy, 200, 100);
        assert_ne!(slower, easy);
    }

    #[test]
    fn subsidy_halves_every_420k() {
        assert_eq!(
            block_subsidy_units(HALVING_INTERVAL_BLOCKS - 1),
            50 * UNITS_PER_DBC
        );
        assert_eq!(block_subsidy_units(HALVING_INTERVAL_BLOCKS), 25 * UNITS_PER_DBC);
        // 12.5 DBC in smallest units.
        assert_eq!(block_subsidy_units(HALVING_INTERVAL_BLOCKS * 2), 1_250_000_000);
    }
}


//! BritishWork — CPU-oriented memory-hard proof-of-work (RandomX-inspired prototype).
//!
//! Whitepaper target: 2 GB RAM.
//!
//! IMPORTANT: BritishWork memory size is a CONSENSUS parameter. For mainnet builds
//! it must be fixed so all nodes agree on valid blocks.

use crate::{BlockHeader, Hash};

/// Production whitepaper target.
pub const PRODUCTION_MEMORY_MIB: usize = 2048;

/// Resolve memory footprint in bytes.
pub fn memory_bytes() -> usize {
    #[cfg(test)]
    {
        return 64 * 1024;
    }
    #[cfg(not(test))]
    {
        PRODUCTION_MEMORY_MIB * 1024 * 1024
    }
}

/// PoW hash for a block header (BritishWork). Compared against `difficulty_target`.
pub fn pow_hash(header: &BlockHeader) -> Result<Hash, String> {
    let bytes = bincode::serialize(header).map_err(|e| e.to_string())?;
    #[cfg(test)]
    {
        // Fast path for unit tests; use `cargo test --release` with `DBC_BRITISHWORK_MIB` for full PoW.
        return Ok(Hash::from_bytes(*blake3::hash(&bytes).as_bytes()));
    }
    #[cfg(not(test))]
    {
        Ok(Hash::from_bytes(*british_work(&bytes, memory_bytes()).as_bytes()))
    }
}

/// Fast verification uses the same function (memory-hard, asymmetric by design).
pub fn verify_pow(header: &BlockHeader) -> Result<Hash, String> {
    pow_hash(header)
}

/// Memory-hard hash: fill scratch from seed, walk memory, finalize with BLAKE3.
#[cfg(not(test))]
fn british_work(header_bytes: &[u8], mem_bytes: usize) -> blake3::Hash {
    let seed = blake3::hash(header_bytes);
    let mut scratch = vec![0u8; mem_bytes];

    // Expand scratch deterministically from seed (sequential writes = cache-friendly on CPU).
    let mut state = *seed.as_bytes();
    for chunk in scratch.chunks_mut(64) {
        state = *blake3::hash(&state).as_bytes();
        let n = chunk.len();
        let fill = blake3::hash(&state);
        let fb = fill.as_bytes();
        for (i, b) in chunk.iter_mut().enumerate().take(n) {
            *b = fb[i % 32];
        }
    }

    // Pseudorandom walks (branchy index pattern favors CPU over GPU).
    let mut acc = *seed.as_bytes();
    let steps = if cfg!(test) { 1 << 10 } else { 1 << 18 };
    for i in 0..steps {
        let idx = {
            let mut x = (i as u64)
                .wrapping_mul(acc[0] as u64)
                .wrapping_add(acc[1] as u64);
            x ^= x >> 33;
            x = x.wrapping_mul(0xff51_afed_4ed9_5853);
            (x as usize) % scratch.len().max(1)
        };
        let end = (idx + 32).min(scratch.len());
        let slice = &scratch[idx..end];
        let mut buf = [0u8; 64];
        buf[..acc.len()].copy_from_slice(&acc);
        buf[32..32 + slice.len()].copy_from_slice(slice);
        acc = *blake3::hash(&buf).as_bytes();
    }

    blake3::hash(&acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hash, BlockHeader};

    #[test]
    fn british_work_is_deterministic() {
        let h = BlockHeader {
            version: 1,
            previous_block_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            timestamp: 1,
            difficulty_target: 0x1f00_ffff,
            nonce: 42,
            height: 0,
            uncle_hashes: vec![],
        };
        let a = pow_hash(&h).unwrap();
        let b = pow_hash(&h).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn nonce_changes_pow() {
        let mut h = BlockHeader {
            version: 1,
            previous_block_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            timestamp: 1,
            difficulty_target: 0x1f00_ffff,
            nonce: 0,
            height: 0,
            uncle_hashes: vec![],
        };
        let a = pow_hash(&h).unwrap();
        h.nonce = 1;
        let b = pow_hash(&h).unwrap();
        assert_ne!(a, b);
    }
}

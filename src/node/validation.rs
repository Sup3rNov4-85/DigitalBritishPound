use k256::schnorr::{signature::Verifier, Signature, VerifyingKey};
use sha3::{Digest, Sha3_256};

use crate::{
    consensus::{block_subsidy_units, MAX_BLOCK_BYTES, COINBASE_MATURITY_BLOCKS},
    crypto::{british_work::pow_hash, wallet::Address},
    node::uncles::{uncle_reward_units, validate_uncles},
    storage::chaindb::ChainDb,
    types::utxo::UTXOSet,
    Block, DbcError, Script, Transaction,
};

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("utxo error: {0}")]
    Utxo(#[from] crate::types::utxo::UtxoError),
    #[error("dbc error: {0}")]
    Dbc(#[from] DbcError),
    #[error("missing coinbase")]
    MissingCoinbase,
    #[error("coinbase must be first transaction")]
    BadCoinbasePosition,
    #[error("invalid script_pubkey")]
    BadScriptPubKey,
    #[error("invalid script_sig")]
    BadScriptSig,
    #[error("signature verification failed")]
    BadSignature,
    #[error("address mismatch for pubkey")]
    AddressMismatch,
    #[error("coinbase immature (requires {COINBASE_MATURITY_BLOCKS} blocks)")]
    CoinbaseImmature,
    #[error("inputs < outputs")]
    ValueCreation,
    #[error("coinbase pays too much: {paid} > {allowed}")]
    CoinbasePaysTooMuch { paid: u64, allowed: u64 },
    #[error("block exceeds max size")]
    BlockTooLarge,
    #[error("merkle root mismatch")]
    BadMerkleRoot,
    #[error("uncle list mismatch")]
    UncleMismatch,
    #[error("too many uncles")]
    TooManyUncles,
    #[error("uncle hash mismatch")]
    UncleHashMismatch,
    #[error("uncle failed pow")]
    UncleBadPow,
    #[error("block failed proof of work")]
    BadBlockPow,
    #[error("uncle height out of range")]
    UncleBadHeight,
    #[error("uncle parent unknown")]
    UncleUnknownParent,
    #[error("duplicate uncle")]
    UncleDuplicate,
}

/// Very small "standard" script format for this prototype:
///
/// - script_pubkey: `[0x14][20-byte address]` (PUSH20)
/// - script_sig: `[sig_len u8][sig bytes][pubkey32]` (x-only schnorr pubkey)
pub fn parse_p2addr_script_pubkey(script: &Script) -> Result<Address, ValidationError> {
    let b = script.as_bytes();
    if b.len() != 21 || b[0] != 0x14 {
        return Err(ValidationError::BadScriptPubKey);
    }
    let mut a = [0u8; 20];
    a.copy_from_slice(&b[1..21]);
    Ok(Address(a))
}

pub fn build_p2addr_script_pubkey(address: Address) -> Script {
    let mut spk = Vec::with_capacity(21);
    spk.push(0x14);
    spk.extend_from_slice(address.as_bytes());
    Script::new(spk)
}

pub fn parse_script_sig(script: &Script) -> Result<(Signature, [u8; 32]), ValidationError> {
    let b = script.as_bytes();
    if b.len() < 1 + 64 + 32 {
        return Err(ValidationError::BadScriptSig);
    }
    let sig_len = b[0] as usize;
    if sig_len != 64 || b.len() != 1 + sig_len + 32 {
        return Err(ValidationError::BadScriptSig);
    }
    let sig = Signature::try_from(&b[1..1 + 64]).map_err(|_| ValidationError::BadScriptSig)?;
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&b[1 + 64..]);
    Ok((sig, pk))
}

pub fn build_script_sig(sig: &Signature, pubkey32: [u8; 32]) -> Script {
    let mut ss = Vec::with_capacity(1 + 64 + 32);
    ss.push(64u8);
    ss.extend_from_slice(sig.to_bytes().as_slice());
    ss.extend_from_slice(&pubkey32);
    Script::new(ss)
}

pub fn address_from_pubkey32(pubkey32: &[u8; 32]) -> Address {
    let sha = Sha3_256::digest(pubkey32);
    let b3 = blake3::hash(&sha);
    let mut out = [0u8; 20];
    out.copy_from_slice(&b3.as_bytes()[..20]);
    Address(out)
}

/// Deterministic signature hash for input `input_index`.
///
/// Prototype choice: hash the transaction with all `script_sig`s empty, plus the input index.
pub fn sighash(tx: &Transaction, input_index: usize) -> Result<[u8; 32], ValidationError> {
    let mut tx2 = tx.clone();
    for inp in &mut tx2.inputs {
        inp.script_sig = Script::new(vec![]);
    }
    let mut bytes = bincode::serialize(&tx2).map_err(|e| ValidationError::Dbc(DbcError::SerialiseError(e.to_string())))?;
    bytes.extend_from_slice(&(input_index as u64).to_le_bytes());
    Ok(*blake3::hash(&bytes).as_bytes())
}

pub struct TxFeeInfo {
    pub fees_units: u64,
    pub input_value_units: u64,
    pub output_value_units: u64,
}

pub fn validate_transaction(utxos: &UTXOSet, tx: &Transaction, current_height: u64) -> Result<TxFeeInfo, ValidationError> {
    if tx.is_coinbase() {
        // Coinbase validity is checked at the block level (subsidy/fees).
        return Ok(TxFeeInfo { fees_units: 0, input_value_units: 0, output_value_units: tx.total_output_value() });
    }

    let mut input_sum = 0u64;
    let output_sum = tx.total_output_value();

    for (i, input) in tx.inputs.iter().enumerate() {
        if input.is_coinbase() {
            return Err(ValidationError::BadScriptSig);
        }

        let utxo = utxos
            .get(&input.previous_output)?
            .ok_or(crate::types::utxo::UtxoError::UnknownUtxo {
                txid: input.previous_output.txid,
                vout: input.previous_output.vout,
            })?;

        // Coinbase maturity.
        if utxo.is_coinbase && !utxo.is_mature(current_height) {
            return Err(ValidationError::CoinbaseImmature);
        }

        // Script check: must match address.
        let expected_addr = parse_p2addr_script_pubkey(&utxo.output.script_pubkey)?;
        let (sig, pubkey32) = parse_script_sig(&input.script_sig)?;
        let derived_addr = address_from_pubkey32(&pubkey32);
        if derived_addr != expected_addr {
            return Err(ValidationError::AddressMismatch);
        }

        let vk = VerifyingKey::from_bytes(&pubkey32).map_err(|_| ValidationError::BadScriptSig)?;
        let msg32 = sighash(tx, i)?;
        vk.verify(&msg32, &sig).map_err(|_| ValidationError::BadSignature)?;

        input_sum = input_sum.saturating_add(utxo.value());
    }

    if input_sum < output_sum {
        return Err(ValidationError::ValueCreation);
    }

    Ok(TxFeeInfo {
        fees_units: input_sum - output_sum,
        input_value_units: input_sum,
        output_value_units: output_sum,
    })
}

pub struct BlockFees {
    pub total_fees_units: u64,
}

pub fn validate_block(
    chain: &ChainDb,
    utxos: &UTXOSet,
    block: &Block,
) -> Result<BlockFees, ValidationError> {
    let height = block.header.height;

    // Max block size (2 MB per whitepaper).
    let size = bincode::serialize(block)
        .map_err(|e| ValidationError::Dbc(DbcError::SerialiseError(e.to_string())))?
        .len();
    if size > MAX_BLOCK_BYTES {
        return Err(ValidationError::BlockTooLarge);
    }

    if !block.verify_merkle_root().map_err(|e| ValidationError::Dbc(e))? {
        return Err(ValidationError::BadMerkleRoot);
    }

    validate_uncles(chain, block)?;
    let uncle_rewards = uncle_reward_units(block)?;

    let coinbase = block.transactions.first().ok_or(ValidationError::MissingCoinbase)?;
    if !coinbase.is_coinbase() {
        return Err(ValidationError::BadCoinbasePosition);
    }

    let mut total_fees = 0u64;
    for tx in block.transactions.iter().skip(1) {
        let info = validate_transaction(utxos, tx, height)?;
        total_fees = total_fees.saturating_add(info.fees_units);
    }

    // Coinbase payout must not exceed subsidy + fees + uncle rewards.
    let allowed = block_subsidy_units(height)
        .saturating_add(total_fees)
        .saturating_add(uncle_rewards);
    let paid = coinbase.total_output_value();
    if paid > allowed {
        return Err(ValidationError::CoinbasePaysTooMuch { paid, allowed });
    }

    Ok(BlockFees {
        total_fees_units: total_fees,
    })
}

/// Verify BritishWork PoW for a block header.
pub fn verify_block_pow(header: &crate::BlockHeader) -> Result<(), ValidationError> {
    let pow = pow_hash(header).map_err(|e| ValidationError::Dbc(DbcError::SerialiseError(e)))?;
    if !crate::hash_meets_target(&pow, header.difficulty_target) {
        return Err(ValidationError::BadBlockPow);
    }
    Ok(())
}


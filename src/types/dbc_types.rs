// ============================================================================
// Digital British Coin (DBC) — Core Blockchain Data Structures
// ============================================================================
//
// Hashing  : BLAKE3  (blake3 crate)
// Serialise: serde + serde_json / bincode compatible
// Encoding : hex strings for human-readable hashes in JSON output
//
// Cargo.toml dependencies required:
//
//   [dependencies]
//   blake3    = "1"
//   serde     = { version = "1", features = ["derive"] }
//   serde_json = "1"
//   hex       = "0.4"
//   thiserror = "1"
// ============================================================================

use serde::{Deserialize, Serialize};

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors that can arise when constructing or validating DBC primitives.
#[derive(Debug)]
pub enum DbcError {
    /// Serialisation failed before hashing.
    SerialiseError(String),
    /// A field violated a structural constraint (e.g. too many uncle hashes).
    ValidationError(String),
}

impl std::fmt::Display for DbcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbcError::SerialiseError(msg) => write!(f, "Serialisation error: {msg}"),
            DbcError::ValidationError(msg) => write!(f, "Validation error: {msg}"),
        }
    }
}

impl std::error::Error for DbcError {}

// ── Hash newtype ─────────────────────────────────────────────────────────────

/// A 32-byte BLAKE3 digest, the universal hash type across all DBC structures.
///
/// Stored as a fixed-size byte array for zero-copy operations; serialises to
/// a lowercase hex string for human-readable formats (JSON) and as raw bytes
/// for binary formats (bincode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hash(
    /// Raw 32-byte digest produced by BLAKE3.
    #[serde(with = "hex_serde")]
    pub [u8; 32],
);

impl Hash {
    /// The all-zeros sentinel used where no previous block exists (genesis).
    pub const ZERO: Hash = Hash([0u8; 32]);

    /// Wrap an existing 32-byte array.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }

    /// Return the inner byte slice.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex representation (64 characters).
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Display for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

// ── Serde helper: hex-encode [u8; 32] ────────────────────────────────────────

/// Private serde module so `Hash` serialises as a hex string in JSON but as
/// raw bytes in binary codecs that don't call `serialize_str`.
mod hex_serde {
    use serde::{de::Error, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str(&hex::encode(bytes))
        } else {
            s.serialize_bytes(bytes)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        if d.is_human_readable() {
            let hex_str = <&str as serde::Deserialize>::deserialize(d)?;
            let decoded = hex::decode(hex_str).map_err(D::Error::custom)?;
            decoded
                .try_into()
                .map_err(|_| D::Error::custom("expected 32-byte hash"))
        } else {
            let bytes = <Vec<u8> as serde::Deserialize>::deserialize(d)?;
            bytes
                .try_into()
                .map_err(|_| D::Error::custom("expected 32-byte hash"))
        }
    }
}

// ── Script type ──────────────────────────────────────────────────────────────

/// Raw locking / unlocking script bytes.
///
/// DBC uses a simple stack-based script for output locking conditions
/// (e.g. pay-to-public-key-hash) and input unlocking conditions
/// (signature + public key). Kept as an opaque `Vec<u8>` here; the script
/// interpreter lives in a separate crate (`dbc-script`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script(
    /// Raw bytecode.
    #[serde(with = "serde_bytes_hex")]
    pub Vec<u8>,
);

impl Script {
    pub fn new(bytes: Vec<u8>) -> Self {
        Script(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Serde helper to hex-encode arbitrary `Vec<u8>` in human-readable formats.
mod serde_bytes_hex {
    use serde::{de::Error, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            s.serialize_str(&hex::encode(bytes))
        } else {
            s.serialize_bytes(bytes)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        if d.is_human_readable() {
            let hex_str = <&str as serde::Deserialize>::deserialize(d)?;
            hex::decode(hex_str).map_err(D::Error::custom)
        } else {
            <Vec<u8> as serde::Deserialize>::deserialize(d)
        }
    }
}

// ============================================================================
// 1. OutPoint — unambiguous reference to a prior transaction output
// ============================================================================

/// Identifies a specific output within a previously confirmed transaction.
///
/// Every `TxInput` (except coinbase inputs) must reference an `OutPoint` to
/// prove it is spending a real, previously unspent output (UTXO).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutPoint {
    /// BLAKE3 hash of the transaction that contains the output being spent.
    ///
    /// For coinbase inputs this is set to `Hash::ZERO` (32 zero bytes) because
    /// there is no prior transaction — the coinbase creates new DBC from thin
    /// air as the block reward.
    pub txid: Hash,

    /// Zero-based index of the specific output within `txid`.
    ///
    /// A transaction may have many outputs; `vout` selects which one this
    /// input is claiming.  `u32::MAX` (0xFFFF_FFFF) is the sentinel for
    /// coinbase inputs alongside `txid == Hash::ZERO`.
    pub vout: u32,
}

impl OutPoint {
    /// Sentinel `OutPoint` used by coinbase transactions.
    pub const COINBASE: OutPoint = OutPoint {
        txid: Hash::ZERO,
        vout: u32::MAX,
    };

    /// Returns `true` if this is a coinbase outpoint.
    pub fn is_coinbase(&self) -> bool {
        self.txid == Hash::ZERO && self.vout == u32::MAX
    }
}

// ============================================================================
// 2. TxInput — spending half of a transaction
// ============================================================================

/// One input to a transaction; claims ownership of a previously unspent output.
///
/// To spend an output the signer must provide a valid `script_sig` that
/// satisfies the `script_pubkey` of the referenced `OutPoint`'s `TxOutput`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxInput {
    /// Reference to the UTXO being consumed by this input.
    ///
    /// Set to `OutPoint::COINBASE` for the first (and only) input of a coinbase
    /// transaction.
    pub previous_output: OutPoint,

    /// Unlocking script (scriptSig / witness data).
    ///
    /// Must satisfy the locking conditions (`script_pubkey`) of the referenced
    /// output.  For P2PKH outputs this typically contains a DER-encoded
    /// ECDSA/Schnorr signature followed by the compressed public key.
    pub script_sig: Script,

    /// Sequence number, enabling relative time-locks (BIP 68-style).
    ///
    /// If a transaction has any input with `sequence < 0xFFFF_FFFE` the
    /// transaction is considered to have opted in to RBF (replace-by-fee).
    /// `0xFFFF_FFFF` disables the sequence lock entirely.
    pub sequence: u32,

    /// Arbitrary data field used exclusively by coinbase inputs.
    ///
    /// For coinbase inputs, miner-supplied bytes go here (e.g. extra nonce,
    /// block height, pool tag).  Must be `None` for all non-coinbase inputs.
    /// Maximum 100 bytes when present.
    pub coinbase_data: Option<Vec<u8>>,
}

impl TxInput {
    /// Construct a standard (non-coinbase) input.
    pub fn new(previous_output: OutPoint, script_sig: Script, sequence: u32) -> Self {
        TxInput {
            previous_output,
            script_sig,
            sequence,
            coinbase_data: None,
        }
    }

    /// Construct a coinbase input for block reward transactions.
    pub fn new_coinbase(data: Vec<u8>) -> Result<Self, DbcError> {
        if data.len() > 100 {
            return Err(DbcError::ValidationError(
                "coinbase_data must not exceed 100 bytes".into(),
            ));
        }
        Ok(TxInput {
            previous_output: OutPoint::COINBASE,
            script_sig: Script::new(vec![]),
            sequence: u32::MAX,
            coinbase_data: Some(data),
        })
    }

    /// Returns `true` when this input is a coinbase.
    pub fn is_coinbase(&self) -> bool {
        self.previous_output.is_coinbase()
    }
}

// ============================================================================
// 3. TxOutput — receiving half of a transaction
// ============================================================================

/// One output of a transaction; creates a new UTXO that a future input can spend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxOutput {
    /// Amount transferred in this output, denominated in the smallest unit
    /// (1 DBC = 10⁸ units; colloquially "pence" per the whitepaper).
    ///
    /// Maximum supply is capped at 42 000 000 DBC (4.2 × 10¹⁵ units), so `u64`
    /// is more than sufficient. A value of `0` is legal (dust / OP_RETURN
    /// outputs) but economically discouraged.
    pub value: u64,

    /// Locking script (scriptPubKey) that encodes the spending conditions.
    ///
    /// Common patterns:
    /// - **P2PKH** — Pay to Public Key Hash (most frequent, ~25 bytes)
    /// - **P2SH**  — Pay to Script Hash (multi-sig, time-locked, etc.)
    /// - **OP_RETURN** — Unspendable output carrying up to 80 bytes of metadata
    ///
    /// The script interpreter validates this against an input's `script_sig`
    /// when the UTXO is spent.
    pub script_pubkey: Script,
}

impl TxOutput {
    pub fn new(value: u64, script_pubkey: Script) -> Self {
        TxOutput {
            value,
            script_pubkey,
        }
    }
}

// ============================================================================
// 4. Transaction
// ============================================================================

/// A DBC transaction: the atomic unit of value transfer on the blockchain.
///
/// A transaction consumes one or more UTXOs (`inputs`) and creates one or more
/// new UTXOs (`outputs`).  The sum of output values must not exceed the sum of
/// input values; the difference is the implicit miner fee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    /// Protocol version number.
    ///
    /// Current version is `1`.  Version `2` enables BIP 68 relative lock-times
    /// when inputs have `sequence < 0xFFFF_FFFE`.  Unknown versions are rejected
    /// by full nodes.
    pub version: u32,

    /// Ordered list of inputs consuming existing UTXOs.
    ///
    /// Must contain at least one input.  The very first transaction in any block
    /// is a *coinbase* transaction whose sole input has `OutPoint::COINBASE`.
    pub inputs: Vec<TxInput>,

    /// Ordered list of outputs creating new UTXOs.
    ///
    /// Must contain at least one output.  Output indices (`vout`) are determined
    /// by position in this vector.
    pub outputs: Vec<TxOutput>,

    /// Absolute lock-time: earliest block height *or* Unix timestamp at which
    /// this transaction may be included in a block.
    ///
    /// - `0`              → no lock, immediately valid.
    /// - `1 ..= 499_999_999` → interpreted as a block **height**.
    /// - `≥ 500_000_000`  → interpreted as a Unix **timestamp** (seconds).
    ///
    /// Lock-time is ignored if all inputs have `sequence == 0xFFFF_FFFF`.
    pub locktime: u32,
}

impl Transaction {
    /// Compute the TXID: the BLAKE3 hash of the canonical serialised transaction.
    ///
    /// Serialisation uses `bincode` (little-endian, fixed-width integers) for
    /// deterministic byte ordering independent of platform endianness.
    ///
    /// The resulting `Hash` is used:
    /// - as the `txid` in `OutPoint` references,
    /// - as a leaf in the block's Merkle tree.
    pub fn compute_hash(&self) -> Result<Hash, DbcError> {
        // Serialise with bincode for a deterministic binary representation.
        let bytes = bincode::serialize(self)
            .map_err(|e| DbcError::SerialiseError(e.to_string()))?;

        let digest = blake3::hash(&bytes);
        Ok(Hash::from_bytes(*digest.as_bytes()))
    }

    /// Returns `true` if this is a coinbase transaction (first tx in a block).
    pub fn is_coinbase(&self) -> bool {
        self.inputs.len() == 1 && self.inputs[0].is_coinbase()
    }

    /// Total value of all outputs in satoshis.
    pub fn total_output_value(&self) -> u64 {
        self.outputs.iter().map(|o| o.value).sum()
    }
}

// ============================================================================
// 5. BlockHeader
// ============================================================================

/// The compact, fixed-size header of a DBC block.
///
/// The header is hashed repeatedly during Proof-of-Work mining; keeping it
/// small (and free of the full transaction list) means nodes can verify PoW
/// without downloading all transactions.
///
/// BLAKE3 is used rather than double-SHA256 (Bitcoin) for its resistance to
/// length-extension attacks and superior throughput on modern hardware.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    /// Block format version, used for consensus rule upgrades.
    ///
    /// Miners signal readiness for soft-forks by setting specific bits in this
    /// field (BIP 9 versionbits style).  Current base version is `1`.
    pub version: u32,

    /// BLAKE3 hash of the *previous* block's header.
    ///
    /// This back-pointer forms the immutable chain: changing any historical
    /// block invalidates every subsequent header hash.  Set to `Hash::ZERO`
    /// only for the genesis block (height 0).
    pub previous_block_hash: Hash,

    /// Root of the binary Merkle tree built from all transaction hashes.
    ///
    /// Committing to all TXIDs in a single 32-byte field lets light clients
    /// (SPV) verify a transaction's inclusion via a Merkle proof without
    /// downloading the full block body.
    pub merkle_root: Hash,

    /// Unix timestamp (seconds since 1970-01-01 00:00:00 UTC) at which the
    /// miner began hashing this header.
    ///
    /// Network nodes reject blocks with timestamps more than two hours in the
    /// future.  The field wraps on 7 February 2106; DBC should adopt a wider
    /// type well before that date.
    pub timestamp: u32,

    /// Compact representation of the current Proof-of-Work target.
    ///
    /// Encodes a 256-bit target in 4 bytes (Bitcoin nBits format): the
    /// top byte is the exponent, the lower 3 bytes are the mantissa.
    /// A valid block header hash must be numerically less than this target.
    pub difficulty_target: u32,

    /// 32-bit counter incremented by the miner during Proof-of-Work.
    ///
    /// The miner iterates `nonce` until `compute_hash()` produces a digest
    /// below `difficulty_target`.  If the full nonce space is exhausted
    /// without a solution, the miner typically updates `timestamp` or the
    /// coinbase extra-nonce and restarts.
    pub nonce: u32,

    /// Distance from the genesis block in the longest chain.
    ///
    /// Genesis block has `height = 0`.  Not strictly required for consensus
    /// (it can be derived from the chain), but including it in the header
    /// allows efficient range queries in block indices.
    pub height: u64,

    /// BLAKE3 hashes of up to **two** uncle (ommer) block headers.
    ///
    /// DBC adopts an uncle / GHOST-style mechanism to reward miners whose
    /// valid blocks were not selected as the main-chain tip (due to network
    /// latency).  Including uncles in the nephew block:
    /// - partially compensates uncle miners,
    /// - increases effective chain security at high block rates,
    /// - reduces selfish-mining incentives.
    ///
    /// Constraints enforced at validation time:
    /// - At most 2 uncle hashes per block.
    /// - Each uncle must be at depth 1–6 below the current block.
    /// - A given uncle hash may appear at most once across the last 7 blocks.
    pub uncle_hashes: Vec<Hash>,
}

impl BlockHeader {
    /// Maximum number of uncle hashes permitted per block.
    pub const MAX_UNCLES: usize = 2;

    /// Validate structural constraints on the header (does not check PoW).
    pub fn validate(&self) -> Result<(), DbcError> {
        if self.uncle_hashes.len() > Self::MAX_UNCLES {
            return Err(DbcError::ValidationError(format!(
                "uncle_hashes length {} exceeds maximum of {}",
                self.uncle_hashes.len(),
                Self::MAX_UNCLES
            )));
        }
        Ok(())
    }
}

// ============================================================================
// 6. Block — header + body
// ============================================================================

/// A complete DBC block: the fundamental unit of the blockchain.
///
/// A block bundles a set of transactions, seals them under the `header`'s
/// Merkle root, and anchors them to the chain via `header.previous_block_hash`.
/// Miners compete to find a `header.nonce` such that the block hash satisfies
/// the current difficulty target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// The compact proof-of-work header (80 bytes on the wire).
    ///
    /// The block hash is defined as `BLAKE3(serialise(header))`.  Changing
    /// any field in the header — including the Merkle root — invalidates the PoW.
    pub header: BlockHeader,

    /// Ordered list of transactions included in this block.
    ///
    /// - `transactions[0]` **must** always be the coinbase transaction.
    /// - Remaining transactions may appear in any miner-chosen order (miners
    ///   typically prioritise by fee-per-byte).
    /// - The Merkle tree is built from `[tx.compute_hash() for tx in transactions]`
    ///   and the root must match `header.merkle_root`.
    pub transactions: Vec<Transaction>,

    /// Full uncle block headers referenced in `header.uncle_hashes`.
    ///
    /// The number of entries here must exactly match `header.uncle_hashes.len()`.
    /// Their hashes must equal the corresponding entries in `header.uncle_hashes`.
    /// Uncle *bodies* (transactions) are **not** included — only the header is
    /// needed to verify the uncle's PoW and lineage.
    pub uncle_blocks: Vec<BlockHeader>,
}

impl Block {
    /// Compute the block hash: BLAKE3 over the serialised `BlockHeader`.
    ///
    /// Only the header is hashed (not the full transaction list) so that:
    /// 1. Light clients can verify PoW without downloading transactions.
    /// 2. The hash can be computed in O(1) space relative to block size.
    ///
    /// The returned `Hash` is used as `previous_block_hash` in the next block
    /// and as the lookup key in block indices.
    pub fn compute_hash(&self) -> Result<Hash, DbcError> {
        // Validate header constraints before hashing.
        self.header.validate()?;

        let bytes = bincode::serialize(&self.header)
            .map_err(|e| DbcError::SerialiseError(e.to_string()))?;

        let digest = blake3::hash(&bytes);
        Ok(Hash::from_bytes(*digest.as_bytes()))
    }

    /// Verify that `header.merkle_root` matches the Merkle root recomputed
    /// from the block's transaction list.
    ///
    /// Returns `Ok(true)` when the roots match, `Ok(false)` on mismatch.
    pub fn verify_merkle_root(&self) -> Result<bool, DbcError> {
        let root = compute_merkle_root(&self.transactions)?;
        Ok(root == self.header.merkle_root)
    }

    /// Returns the coinbase transaction, which must always be `transactions[0]`.
    pub fn coinbase(&self) -> Option<&Transaction> {
        self.transactions.first()
    }

    /// Total fees available to the miner (sum of all non-coinbase transaction
    /// implicit fees).  Requires access to the UTXO set to compute input values;
    /// this helper only sums output values as a lower-bound approximation.
    pub fn transaction_count(&self) -> usize {
        self.transactions.len()
    }
}

// ============================================================================
// 7. Merkle tree helper
// ============================================================================

/// Compute a BLAKE3-based binary Merkle root from a slice of transactions.
///
/// Algorithm (identical to Bitcoin's except SHA256d → BLAKE3):
/// 1. Compute the TXID (BLAKE3 hash) of each transaction → leaf layer.
/// 2. Pair adjacent leaves; if odd count, duplicate the last leaf.
/// 3. Hash each pair: `BLAKE3(left_hash ‖ right_hash)`.
/// 4. Repeat until a single root hash remains.
///
/// Returns `Hash::ZERO` for an empty transaction list.
pub fn compute_merkle_root(transactions: &[Transaction]) -> Result<Hash, DbcError> {
    if transactions.is_empty() {
        return Ok(Hash::ZERO);
    }

    // Build leaf layer from TXIDs.
    let mut layer: Vec<Hash> = transactions
        .iter()
        .map(|tx| tx.compute_hash())
        .collect::<Result<Vec<_>, _>>()?;

    // Iteratively reduce until one root remains.
    while layer.len() > 1 {
        // Duplicate last element if the layer has an odd number of nodes.
        if layer.len() % 2 != 0 {
            let last = *layer.last().unwrap();
            layer.push(last);
        }

        layer = layer
            .chunks_exact(2)
            .map(|pair| {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(pair[0].as_bytes());
                buf[32..].copy_from_slice(pair[1].as_bytes());
                let digest = blake3::hash(&buf);
                Hash::from_bytes(*digest.as_bytes())
            })
            .collect();
    }

    Ok(layer[0])
}

// ============================================================================
// 8. Compact difficulty target helpers
// ============================================================================

/// Decode a compact `difficulty_target` (nBits) into a full 32-byte target.
///
/// Format: `exponent = target_bytes[0]`, `mantissa = target_bytes[1..=3]`.
/// The expanded target is `mantissa * 256^(exponent - 3)`.
pub fn expand_compact_target(compact: u32) -> [u8; 32] {
    let exponent = (compact >> 24) as usize;
    let mantissa = compact & 0x00FF_FFFF;

    let mut target = [0u8; 32];
    if exponent == 0 || exponent > 32 {
        return target; // degenerate / overflow
    }

    // Write the 3 mantissa bytes at the correct offset.
    let base = exponent.saturating_sub(3);
    if base < 32 {
        let be = mantissa.to_be_bytes();
        for (i, &b) in be[1..].iter().enumerate() {
            let idx = 32 - base - 3 + i;
            if idx < 32 {
                target[idx] = b;
            }
        }
    }
    target
}

/// Returns `true` if `hash` satisfies the given compact `difficulty_target`
/// (i.e. the hash value is numerically less than the expanded target).
pub fn hash_meets_target(hash: &Hash, difficulty_target: u32) -> bool {
    let target = expand_compact_target(difficulty_target);
    // Compare big-endian: first differing byte decides.
    hash.as_bytes() < &target
}

// ============================================================================
// 9. Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn dummy_script() -> Script {
        Script::new(vec![0x76, 0xa9, 0x14]) // OP_DUP OP_HASH160 OP_PUSH20 prefix
    }

    fn coinbase_tx(height: u64) -> Transaction {
        let cb_data = height.to_le_bytes().to_vec();
        Transaction {
            version: 1,
            inputs: vec![TxInput::new_coinbase(cb_data).unwrap()],
            outputs: vec![TxOutput::new(50_0000_0000, dummy_script())],
            locktime: 0,
        }
    }

    fn simple_tx() -> Transaction {
        let outpoint = OutPoint {
            txid: Hash::ZERO,
            vout: 0,
        };
        Transaction {
            version: 1,
            inputs: vec![TxInput::new(outpoint, dummy_script(), u32::MAX)],
            outputs: vec![TxOutput::new(49_9999_0000, dummy_script())],
            locktime: 0,
        }
    }

    fn genesis_header(merkle_root: Hash) -> BlockHeader {
        BlockHeader {
            version: 1,
            previous_block_hash: Hash::ZERO,
            merkle_root,
            timestamp: 1_700_000_000,
            difficulty_target: 0x1d00_ffff,
            nonce: 0,
            height: 0,
            uncle_hashes: vec![],
        }
    }

    // ── Hash ─────────────────────────────────────────────────────────────────

    #[test]
    fn hash_zero_is_32_zero_bytes() {
        assert_eq!(Hash::ZERO.as_bytes(), &[0u8; 32]);
    }

    #[test]
    fn hash_display_is_64_hex_chars() {
        assert_eq!(Hash::ZERO.to_hex().len(), 64);
    }

    // ── OutPoint ─────────────────────────────────────────────────────────────

    #[test]
    fn coinbase_outpoint_detected() {
        assert!(OutPoint::COINBASE.is_coinbase());
        let normal = OutPoint {
            txid: Hash::ZERO,
            vout: 0,
        };
        assert!(!normal.is_coinbase());
    }

    // ── TxInput ──────────────────────────────────────────────────────────────

    #[test]
    fn coinbase_input_too_long_rejected() {
        let long_data = vec![0u8; 101];
        assert!(TxInput::new_coinbase(long_data).is_err());
    }

    #[test]
    fn coinbase_input_max_size_accepted() {
        let ok_data = vec![0u8; 100];
        assert!(TxInput::new_coinbase(ok_data).is_ok());
    }

    // ── Transaction hashing ──────────────────────────────────────────────────

    #[test]
    fn transaction_hash_is_deterministic() {
        let tx = simple_tx();
        let h1 = tx.compute_hash().unwrap();
        let h2 = tx.compute_hash().unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_transactions_have_different_hashes() {
        let tx1 = coinbase_tx(1);
        let tx2 = coinbase_tx(2);
        assert_ne!(tx1.compute_hash().unwrap(), tx2.compute_hash().unwrap());
    }

    // ── Merkle root ──────────────────────────────────────────────────────────

    #[test]
    fn merkle_root_empty_is_zero() {
        assert_eq!(compute_merkle_root(&[]).unwrap(), Hash::ZERO);
    }

    #[test]
    fn merkle_root_single_tx_equals_txid() {
        let tx = coinbase_tx(0);
        let txid = tx.compute_hash().unwrap();
        let root = compute_merkle_root(&[tx]).unwrap();
        assert_eq!(root, txid);
    }

    #[test]
    fn merkle_root_two_txs_is_consistent() {
        let txs = vec![coinbase_tx(0), simple_tx()];
        let r1 = compute_merkle_root(&txs).unwrap();
        let r2 = compute_merkle_root(&txs).unwrap();
        assert_eq!(r1, r2);
    }

    // ── Block hashing ────────────────────────────────────────────────────────

    #[test]
    fn block_hash_is_deterministic() {
        let txs = vec![coinbase_tx(0)];
        let root = compute_merkle_root(&txs).unwrap();
        let block = Block {
            header: genesis_header(root),
            transactions: txs,
            uncle_blocks: vec![],
        };
        let h1 = block.compute_hash().unwrap();
        let h2 = block.compute_hash().unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn block_merkle_root_verifies() {
        let txs = vec![coinbase_tx(0), simple_tx()];
        let root = compute_merkle_root(&txs).unwrap();
        let block = Block {
            header: genesis_header(root),
            transactions: txs,
            uncle_blocks: vec![],
        };
        assert!(block.verify_merkle_root().unwrap());
    }

    #[test]
    fn block_with_bad_merkle_root_fails_verify() {
        let txs = vec![coinbase_tx(0)];
        let mut header = genesis_header(Hash::ZERO); // deliberately wrong root
        header.merkle_root = Hash::ZERO;
        let block = Block {
            header,
            transactions: txs,
            uncle_blocks: vec![],
        };
        assert!(!block.verify_merkle_root().unwrap());
    }

    // ── Uncle hash validation ────────────────────────────────────────────────

    #[test]
    fn too_many_uncle_hashes_rejected() {
        let mut header = genesis_header(Hash::ZERO);
        header.uncle_hashes = vec![Hash::ZERO, Hash::ZERO, Hash::ZERO]; // 3 > max 2
        assert!(header.validate().is_err());
    }

    #[test]
    fn two_uncle_hashes_accepted() {
        let mut header = genesis_header(Hash::ZERO);
        header.uncle_hashes = vec![Hash::ZERO, Hash::ZERO];
        assert!(header.validate().is_ok());
    }

    // ── JSON round-trip ──────────────────────────────────────────────────────

    #[test]
    fn transaction_json_round_trip() {
        let tx = simple_tx();
        let json = serde_json::to_string(&tx).unwrap();
        let recovered: Transaction = serde_json::from_str(&json).unwrap();
        assert_eq!(tx, recovered);
    }

    #[test]
    fn block_json_round_trip() {
        let txs = vec![coinbase_tx(0)];
        let root = compute_merkle_root(&txs).unwrap();
        let block = Block {
            header: genesis_header(root),
            transactions: txs,
            uncle_blocks: vec![],
        };
        let json = serde_json::to_string_pretty(&block).unwrap();
        let recovered: Block = serde_json::from_str(&json).unwrap();
        assert_eq!(block, recovered);
    }
}

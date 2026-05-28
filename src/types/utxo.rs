// ============================================================================
// Digital British Coin (DBC) — UTXO Set backed by RocksDB
// ============================================================================
//
// Design overview
// ───────────────
// The UTXO set is the canonical view of "all spendable coins right now".
// Every full node maintains this database so it can validate new transactions
// without scanning the entire chain history.
//
// Storage layout (RocksDB key → value)
// ─────────────────────────────────────
//   Key  : OutPoint encoded as 36 bytes  [txid: 32 bytes | vout: 4 bytes BE]
//   Value: Utxo  encoded via bincode
//
// This gives O(log n) lookup, insertion, and deletion, with RocksDB's
// LSM-tree providing efficient bulk-write performance during IBD
// (Initial Block Download).
//
// Atomicity
// ─────────
// apply_block / revert_block both use a RocksDB WriteBatch so the entire
// state transition is atomic: a crash mid-way leaves the database unchanged.
//
// Undo records
// ────────────
// Spending a UTXO destroys it — there is no way to reconstruct a spent
// output from the spending transaction alone (we only have the OutPoint, not
// the original value / script).  apply_block therefore returns an UndoBlock
// containing every UTXO it deleted.  revert_block takes that UndoBlock and
// exactly inverts the forward pass.
// ============================================================================

use std::path::Path;

use rocksdb::{WriteBatch, DB};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Pull the types defined in lib.rs / dbc_types into scope.
use crate::{Block, Hash, OutPoint, TxOutput};

// ── Error type ───────────────────────────────────────────────────────────────

/// Every error that can arise from UTXO-set operations.
#[derive(Debug, Error)]
pub enum UtxoError {
    // ── Storage layer ─────────────────────────────────────────────────────

    /// RocksDB returned an error (I/O, corruption, lock contention, …).
    #[error("RocksDB error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    /// Serialisation to bytes failed (should never happen with valid data).
    #[error("Failed to serialise value: {0}")]
    Serialise(String),

    /// Deserialisation from stored bytes failed (indicates DB corruption).
    #[error("Failed to deserialise value from database: {0}")]
    Deserialise(String),

    // ── Consensus / validation ─────────────────────────────────────────────

    /// An input references an OutPoint that does not exist in the UTXO set.
    /// This means either a double-spend attempt or a reference to a
    /// non-existent transaction.
    #[error("Input references unknown UTXO: txid={txid} vout={vout}")]
    UnknownUtxo { txid: Hash, vout: u32 },

    /// A coinbase transaction is missing from position 0 of the block, or
    /// position 0 contains a non-coinbase transaction.
    #[error("Block coinbase is malformed: {reason}")]
    MalformedCoinbase { reason: String },

    /// The UndoBlock passed to revert_block does not match the block being
    /// reverted (e.g. wrong number of undo entries).
    #[error("UndoBlock is inconsistent with the block being reverted")]
    UndoMismatch,

    /// Attempted to insert an OutPoint that already exists in the UTXO set.
    /// This would indicate a duplicate transaction (forbidden by consensus).
    #[error("Duplicate OutPoint: txid={txid} vout={vout}")]
    DuplicateUtxo { txid: Hash, vout: u32 },
}

// ── UTXO ─────────────────────────────────────────────────────────────────────

/// A single Unspent Transaction Output: the minimal data needed to validate
/// a future spending transaction and to tally the spendable coin supply.
///
/// Stored in RocksDB keyed by its `OutPoint` (txid + vout).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Utxo {
    /// The actual output: locking script and satoshi value.
    ///
    /// This is copied verbatim from `Transaction.outputs[vout]` at the time
    /// the creating transaction was confirmed.
    pub output: TxOutput,

    /// Block height at which this output was created (confirmed).
    ///
    /// Used to enforce the **coinbase maturity rule**: a coinbase output
    /// cannot be spent until 100 blocks have been built on top of the block
    /// that contains it.  Regular outputs have no maturity restriction.
    pub height: u64,

    /// `true` if this output was created by a coinbase transaction.
    ///
    /// Coinbase outputs carry the block reward + fees.  They are subject to
    /// the 100-block maturity rule (see `height`).
    pub is_coinbase: bool,
}

impl Utxo {
    /// Construct a new UTXO from its component parts.
    pub fn new(output: TxOutput, height: u64, is_coinbase: bool) -> Self {
        Utxo {
            output,
            height,
            is_coinbase,
        }
    }

    /// Convenience: the satoshi value of this output.
    pub fn value(&self) -> u64 {
        self.output.value
    }

    /// Returns `true` if this coinbase output has reached the maturity threshold.
    ///
    /// Always returns `true` for non-coinbase outputs.
    pub fn is_mature(&self, current_height: u64) -> bool {
        let required = coinbase_maturity_blocks();
        if self.is_coinbase {
            current_height.saturating_sub(self.height) >= required
        } else {
            true
        }
    }
}

fn coinbase_maturity_blocks() -> u64 {
    #[cfg(test)]
    {
        return 2;
    }
    #[cfg(not(test))]
    {
        crate::consensus::COINBASE_MATURITY_BLOCKS
    }
}

// ── Undo records ─────────────────────────────────────────────────────────────

/// A single spent UTXO record used to reverse a block application.
///
/// When apply_block removes a UTXO from the set it saves it here so that
/// revert_block can restore it exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpentOutput {
    /// The OutPoint that was consumed (the key that was deleted).
    pub outpoint: OutPoint,
    /// The full UTXO value that existed at that key before deletion.
    pub utxo: Utxo,
}

/// All the information required to completely reverse a single applied block.
///
/// Produced by `UTXOSet::apply_block` and consumed by `UTXOSet::revert_block`.
/// Callers must persist this alongside the block if they want to support chain
/// reorganisation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoBlock {
    /// The hash of the block this undo record belongs to.
    pub block_hash: Hash,

    /// Every UTXO that was *consumed* (deleted) during `apply_block`, in the
    /// order they were spent.  revert_block re-inserts each one.
    pub spent_outputs: Vec<SpentOutput>,
}

// ── Key encoding ─────────────────────────────────────────────────────────────

/// Encode an `OutPoint` as a 36-byte RocksDB key.
///
/// Layout: `[txid: 32 bytes][vout: 4 bytes big-endian]`
///
/// Big-endian `vout` keeps keys with the same `txid` lexicographically
/// adjacent, which is cache-friendly for multi-output transactions.
#[inline]
fn encode_key(outpoint: &OutPoint) -> [u8; 36] {
    let mut key = [0u8; 36];
    key[..32].copy_from_slice(outpoint.txid.as_bytes());
    key[32..].copy_from_slice(&outpoint.vout.to_be_bytes());
    key
}

/// Decode a 36-byte RocksDB key back into an `OutPoint`.
#[inline]
fn decode_key(key: &[u8; 36]) -> OutPoint {
    let mut txid_bytes = [0u8; 32];
    txid_bytes.copy_from_slice(&key[..32]);
    let vout = u32::from_be_bytes(key[32..].try_into().unwrap());
    OutPoint {
        txid: Hash::from_bytes(txid_bytes),
        vout,
    }
}

/// Serialise a `Utxo` to bytes using bincode (little-endian, compact).
fn serialise_utxo(utxo: &Utxo) -> Result<Vec<u8>, UtxoError> {
    bincode::serialize(utxo).map_err(|e| UtxoError::Serialise(e.to_string()))
}

/// Deserialise a `Utxo` from bytes previously written by `serialise_utxo`.
fn deserialise_utxo(bytes: &[u8]) -> Result<Utxo, UtxoError> {
    bincode::deserialize(bytes).map_err(|e| UtxoError::Deserialise(e.to_string()))
}

// ── UTXOSet ───────────────────────────────────────────────────────────────────

/// The full set of all Unspent Transaction Outputs, persisted in RocksDB.
///
/// # Thread safety
///
/// RocksDB's underlying `DB` handle is `Send + Sync`, meaning `UTXOSet` can
/// be shared across threads behind an `Arc<Mutex<UTXOSet>>` or by calling the
/// read-only methods (`get`, `contains`) concurrently without a mutex.
///
/// Mutating methods (`insert`, `remove`, `apply_block`, `revert_block`) should
/// be serialised by the caller.
///
/// # Example
///
/// ```rust,ignore
/// let utxo_set = UTXOSet::new("/data/dbc/utxo")?;
/// let undo = utxo_set.apply_block(&block)?;
/// // … later, during a reorg:
/// utxo_set.revert_block(&block, &undo)?;
/// ```
pub struct UTXOSet {
    /// Handle to the open RocksDB instance.
    db: DB,
}

impl UTXOSet {
    // ── Lifecycle ─────────────────────────────────────────────────────────

    /// Open (or create) the UTXO database at `path`.
    ///
    /// RocksDB creates the directory and all necessary files if they do not
    /// exist.  If the database already exists it is opened in read-write mode.
    ///
    /// # Errors
    ///
    /// Returns `UtxoError::RocksDb` if the database cannot be opened (e.g.
    /// another process holds the lock, the path is not writable, or the
    /// on-disk format is incompatible with the linked RocksDB version).
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, UtxoError> {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);

        // Tune for UTXO workload: random point lookups + sequential bulk writes
        // during IBD (Initial Block Download).
        opts.set_max_open_files(512);
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64 MiB memtable
        opts.set_max_write_buffer_number(3);
        opts.set_target_file_size_base(64 * 1024 * 1024);
        opts.set_bloom_locality(1); // bloom filter per SST for point lookups

        let db = DB::open(&opts, path)?;
        Ok(UTXOSet { db })
    }

    /// Open the UTXO database in **read-only** mode.
    ///
    /// Useful for explorer / RPC threads that must not modify state.
    pub fn open_read_only<P: AsRef<Path>>(path: P) -> Result<Self, UtxoError> {
        let opts = rocksdb::Options::default();
        let db = DB::open_for_read_only(&opts, path, false)?;
        Ok(UTXOSet { db })
    }

    // ── Point operations ──────────────────────────────────────────────────

    /// Look up a UTXO by its `OutPoint`.
    ///
    /// Returns `Ok(Some(utxo))` if found, `Ok(None)` if not present.
    ///
    /// # Errors
    ///
    /// `UtxoError::RocksDb`      — storage I/O failure.
    /// `UtxoError::Deserialise`  — on-disk bytes are corrupted / incompatible.
    pub fn get(&self, outpoint: &OutPoint) -> Result<Option<Utxo>, UtxoError> {
        let key = encode_key(outpoint);
        match self.db.get(key)? {
            Some(bytes) => Ok(Some(deserialise_utxo(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Insert a new UTXO into the set.
    ///
    /// # Errors
    ///
    /// `UtxoError::DuplicateUtxo` — the OutPoint already exists.  Callers
    /// that want upsert semantics should call `remove` first, or use the
    /// batch-write methods in `apply_block`.
    pub fn insert(&self, outpoint: &OutPoint, utxo: &Utxo) -> Result<(), UtxoError> {
        // Guard against duplicate UTXOs (would indicate a consensus bug).
        if self.contains(outpoint)? {
            return Err(UtxoError::DuplicateUtxo {
                txid: outpoint.txid,
                vout: outpoint.vout,
            });
        }
        let key = encode_key(outpoint);
        let value = serialise_utxo(utxo)?;
        self.db.put(key, value)?;
        Ok(())
    }

    /// Remove a UTXO from the set and return it.
    ///
    /// Returns the removed UTXO so callers can record it in undo data.
    ///
    /// # Errors
    ///
    /// `UtxoError::UnknownUtxo` — the OutPoint is not in the set (double-spend
    /// or reference to non-existent output).
    pub fn remove(&self, outpoint: &OutPoint) -> Result<Utxo, UtxoError> {
        let utxo = self
            .get(outpoint)?
            .ok_or(UtxoError::UnknownUtxo {
                txid: outpoint.txid,
                vout: outpoint.vout,
            })?;
        let key = encode_key(outpoint);
        self.db.delete(key)?;
        Ok(utxo)
    }

    /// Returns `true` if the OutPoint is present in the UTXO set.
    ///
    /// Slightly cheaper than `get` because it avoids deserialising the value.
    pub fn contains(&self, outpoint: &OutPoint) -> Result<bool, UtxoError> {
        let key = encode_key(outpoint);
        // `key_may_exist` is a fast bloom-filter check; a `true` result must
        // be confirmed with a full `get`.  A `false` result is definitive.
        if !self.db.key_may_exist(&key) {
            return Ok(false);
        }
        Ok(self.db.get(key)?.is_some())
    }

    // ── Block-level atomic operations ─────────────────────────────────────

    /// Apply a confirmed block to the UTXO set, atomically.
    ///
    /// Processing order for each transaction (coinbase first):
    ///   1. **Coinbase** — skip inputs (they create DBC from nothing), add
    ///      all outputs as new UTXOs flagged `is_coinbase = true`.
    ///   2. **Regular txs** — remove each input's referenced UTXO (spending
    ///      it), then add each output as a new UTXO.
    ///
    /// The entire set of deletions and insertions is written as a single
    /// RocksDB `WriteBatch`, so the operation is atomic: if the process
    /// crashes mid-way the database is unchanged.
    ///
    /// # Returns
    ///
    /// An `UndoBlock` containing every UTXO that was spent (deleted).  The
    /// caller is responsible for persisting this alongside the block so that
    /// `revert_block` can undo the state transition during a reorg.
    ///
    /// # Errors
    ///
    /// `UtxoError::MalformedCoinbase` — `transactions[0]` is missing or is
    /// not a coinbase transaction.
    ///
    /// `UtxoError::UnknownUtxo` — a non-coinbase input references an OutPoint
    /// that is not in the current UTXO set.
    ///
    /// `UtxoError::DuplicateUtxo` — a transaction output has the same
    /// OutPoint as an existing UTXO (duplicate txid).
    pub fn apply_block(&self, block: &Block) -> Result<UndoBlock, UtxoError> {
        // ── Validate coinbase ──────────────────────────────────────────────
        let coinbase = block
            .transactions
            .first()
            .ok_or_else(|| UtxoError::MalformedCoinbase {
                reason: "block has no transactions".into(),
            })?;

        if !coinbase.is_coinbase() {
            return Err(UtxoError::MalformedCoinbase {
                reason: "transactions[0] is not a coinbase transaction".into(),
            });
        }

        let block_height = block.header.height;

        // ── First pass: validate all inputs before writing anything ────────
        //
        // We collect the spent UTXOs here (to build the UndoBlock and to
        // populate the WriteBatch).  Doing this in a read-only pass before
        // any writes means a validation failure leaves the DB untouched even
        // without the WriteBatch.
        let mut spent_outputs: Vec<SpentOutput> = Vec::new();

        for tx in block.transactions.iter().skip(1) {
            // skip coinbase — its inputs reference no UTXOs
            for input in &tx.inputs {
                let utxo = self
                    .get(&input.previous_output)?
                    .ok_or(UtxoError::UnknownUtxo {
                        txid: input.previous_output.txid,
                        vout: input.previous_output.vout,
                    })?;
                spent_outputs.push(SpentOutput {
                    outpoint: input.previous_output.clone(),
                    utxo,
                });
            }
        }

        // ── Second pass: build WriteBatch ──────────────────────────────────
        let mut batch = WriteBatch::default();

        // Delete all spent UTXOs.
        for spent in &spent_outputs {
            let key = encode_key(&spent.outpoint);
            batch.delete(key);
        }

        // Add coinbase outputs.
        let coinbase_txid = coinbase.compute_hash().map_err(|e| {
            UtxoError::Serialise(format!("failed to hash coinbase tx: {e}"))
        })?;

        for (vout, output) in coinbase.outputs.iter().enumerate() {
            let outpoint = OutPoint {
                txid: coinbase_txid,
                vout: vout as u32,
            };
            let utxo = Utxo::new(output.clone(), block_height, true);
            let key = encode_key(&outpoint);
            let value = serialise_utxo(&utxo)?;
            batch.put(key, value);
        }

        // Add regular transaction outputs.
        for tx in block.transactions.iter().skip(1) {
            let txid = tx.compute_hash().map_err(|e| {
                UtxoError::Serialise(format!("failed to hash tx: {e}"))
            })?;

            for (vout, output) in tx.outputs.iter().enumerate() {
                let outpoint = OutPoint {
                    txid,
                    vout: vout as u32,
                };

                // Detect duplicate OutPoints before batching.
                // (WriteBatch silently overwrites; we want an explicit error.)
                if self.contains(&outpoint)? {
                    return Err(UtxoError::DuplicateUtxo {
                        txid: outpoint.txid,
                        vout: outpoint.vout,
                    });
                }

                let utxo = Utxo::new(output.clone(), block_height, false);
                let key = encode_key(&outpoint);
                let value = serialise_utxo(&utxo)?;
                batch.put(key, value);
            }
        }

        // ── Atomic commit ──────────────────────────────────────────────────
        let mut write_opts = rocksdb::WriteOptions::default();
        write_opts.set_sync(true); // fsync before returning — durability guarantee
        self.db.write_opt(batch, &write_opts)?;

        // ── Compute block hash for the UndoBlock ───────────────────────────
        let block_hash = block.compute_hash().map_err(|e| {
            UtxoError::Serialise(format!("failed to hash block: {e}"))
        })?;

        Ok(UndoBlock {
            block_hash,
            spent_outputs,
        })
    }

    /// Revert a previously applied block, restoring the UTXO set to its
    /// state before `apply_block` was called.
    ///
    /// This is the inverse of `apply_block` and is used during chain
    /// reorganisations when the node switches to a heavier fork.
    ///
    /// Processing order (exact reverse of apply_block):
    ///   1. **Remove** all outputs created by the block's transactions.
    ///   2. **Re-insert** all UTXOs that were consumed by the block's inputs
    ///      (sourced from `undo`).
    ///
    /// As with `apply_block`, all changes are written atomically via a
    /// RocksDB `WriteBatch`.
    ///
    /// # Errors
    ///
    /// `UtxoError::UndoMismatch` — the number of spent-output records in
    /// `undo` does not match the number of inputs in the block's non-coinbase
    /// transactions.  This indicates the caller passed the wrong UndoBlock.
    pub fn revert_block(&self, block: &Block, undo: &UndoBlock) -> Result<(), UtxoError> {
        // ── Sanity-check undo record count ─────────────────────────────────
        let expected_spent: usize = block
            .transactions
            .iter()
            .skip(1) // skip coinbase
            .map(|tx| tx.inputs.len())
            .sum();

        if undo.spent_outputs.len() != expected_spent {
            return Err(UtxoError::UndoMismatch);
        }

        let mut batch = WriteBatch::default();

        // ── Remove outputs created by this block ───────────────────────────
        //
        // Coinbase outputs.
        let coinbase = block.transactions.first().ok_or_else(|| UtxoError::MalformedCoinbase {
            reason: "block has no transactions during revert".into(),
        })?;

        let coinbase_txid = coinbase.compute_hash().map_err(|e| {
            UtxoError::Serialise(format!("failed to hash coinbase tx: {e}"))
        })?;

        for vout in 0..coinbase.outputs.len() as u32 {
            let outpoint = OutPoint { txid: coinbase_txid, vout };
            batch.delete(encode_key(&outpoint));
        }

        // Regular transaction outputs.
        for tx in block.transactions.iter().skip(1) {
            let txid = tx.compute_hash().map_err(|e| {
                UtxoError::Serialise(format!("failed to hash tx: {e}"))
            })?;

            for vout in 0..tx.outputs.len() as u32 {
                let outpoint = OutPoint { txid, vout };
                batch.delete(encode_key(&outpoint));
            }
        }

        // ── Re-insert UTXOs that were spent by this block ─────────────────
        for spent in &undo.spent_outputs {
            let key = encode_key(&spent.outpoint);
            let value = serialise_utxo(&spent.utxo)?;
            batch.put(key, value);
        }

        // ── Atomic commit ──────────────────────────────────────────────────
        let mut write_opts = rocksdb::WriteOptions::default();
        write_opts.set_sync(true);
        self.db.write_opt(batch, &write_opts)?;

        Ok(())
    }

    // ── Diagnostics ───────────────────────────────────────────────────────

    /// Iterate over every UTXO in the set, calling `f` for each one.
    ///
    /// Primarily intended for diagnostics, snapshots, and testing.  Not
    /// suitable for production use on a fully synced node (millions of UTXOs).
    ///
    /// Iteration is in lexicographic key order (txid bytes, then vout BE).
    pub fn for_each<F>(&self, mut f: F) -> Result<(), UtxoError>
    where
        F: FnMut(OutPoint, Utxo) -> Result<(), UtxoError>,
    {
        let iter = self.db.iterator(rocksdb::IteratorMode::Start);
        for item in iter {
            let (key_bytes, value_bytes) = item?;
            if key_bytes.len() != 36 {
                return Err(UtxoError::Deserialise(format!(
                    "unexpected key length {} (expected 36)",
                    key_bytes.len()
                )));
            }
            let key_arr: [u8; 36] = key_bytes.as_ref().try_into().unwrap();
            let outpoint = decode_key(&key_arr);
            let utxo = deserialise_utxo(&value_bytes)?;
            f(outpoint, utxo)?;
        }
        Ok(())
    }

    /// Return the total number of UTXOs currently in the set.
    ///
    /// Computed by a full scan — O(n).  Cache the result externally if you
    /// need it frequently.
    pub fn len(&self) -> Result<usize, UtxoError> {
        let mut count = 0usize;
        self.for_each(|_, _| {
            count += 1;
            Ok(())
        })?;
        Ok(count)
    }

    /// Returns `true` if the UTXO set contains no entries.
    pub fn is_empty(&self) -> Result<bool, UtxoError> {
        Ok(self.len()? == 0)
    }

    /// Total spendable coin supply in satoshis — sum of all UTXO values.
    ///
    /// Full scan — O(n).
    pub fn total_supply(&self) -> Result<u64, UtxoError> {
        let mut total = 0u64;
        self.for_each(|_, utxo| {
            total = total.saturating_add(utxo.value());
            Ok(())
        })?;
        Ok(total)
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Script, Transaction, TxInput, TxOutput};
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    // Each test gets its own temp DB path to avoid cross-test interference.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    fn tmp_path() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("dbc_utxo_test_{n}_{}", std::process::id()))
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn dummy_script() -> Script {
        Script::new(vec![0x76, 0xa9, 0x14])
    }

    fn make_output(value: u64) -> TxOutput {
        TxOutput::new(value, dummy_script())
    }

    fn make_utxo(value: u64, height: u64, is_coinbase: bool) -> Utxo {
        Utxo::new(make_output(value), height, is_coinbase)
    }

    fn make_outpoint(vout: u32) -> OutPoint {
        OutPoint {
            txid: Hash::ZERO,
            vout,
        }
    }

    /// Build a minimal valid block at a given height with one coinbase and
    /// zero regular transactions.
    fn make_coinbase_block(height: u64) -> Block {
        use crate::{BlockHeader, compute_merkle_root};

        let cb_data = height.to_le_bytes().to_vec();
        let coinbase = Transaction {
            version: 1,
            inputs: vec![TxInput::new_coinbase(cb_data).unwrap()],
            outputs: vec![make_output(50_0000_0000)],
            locktime: 0,
        };

        let txs = vec![coinbase];
        let merkle_root = compute_merkle_root(&txs).unwrap();

        Block {
            header: BlockHeader {
                version: 1,
                previous_block_hash: Hash::ZERO,
                merkle_root,
                timestamp: 1_700_000_000 + height as u32,
                difficulty_target: 0x1d00_ffff,
                nonce: 0,
                height,
                uncle_hashes: vec![],
            },
            transactions: txs,
            uncle_blocks: vec![],
        }
    }

    // ── UTXO maturity ────────────────────────────────────────────────────────

    #[test]
    fn coinbase_maturity_rule() {
        let utxo = make_utxo(100, 100, true);
        // In test builds maturity is 2 blocks; in production it is 100.
        assert!(!utxo.is_mature(101));
        assert!(utxo.is_mature(102));
    }

    #[test]
    fn regular_output_always_mature() {
        let utxo = make_utxo(100, 0, false);
        assert!(utxo.is_mature(0));
        assert!(utxo.is_mature(1));
    }

    // ── Key encoding round-trip ───────────────────────────────────────────────

    #[test]
    fn key_encode_decode_roundtrip() {
        let outpoint = OutPoint {
            txid: Hash::from_bytes([0xab; 32]),
            vout: 42,
        };
        let key = encode_key(&outpoint);
        let decoded = decode_key(&key);
        assert_eq!(outpoint.txid, decoded.txid);
        assert_eq!(outpoint.vout, decoded.vout);
    }

    #[test]
    fn key_ordering_same_txid() {
        // vout=1 key must be lexicographically less than vout=2 (BE encoding)
        let op1 = OutPoint { txid: Hash::ZERO, vout: 1 };
        let op2 = OutPoint { txid: Hash::ZERO, vout: 2 };
        assert!(encode_key(&op1) < encode_key(&op2));
    }

    // ── Basic CRUD ────────────────────────────────────────────────────────────

    #[test]
    fn insert_get_remove() {
        let db = UTXOSet::new(tmp_path()).unwrap();
        let op = make_outpoint(0);
        let utxo = make_utxo(1_000, 10, false);

        assert!(!db.contains(&op).unwrap());
        db.insert(&op, &utxo).unwrap();
        assert!(db.contains(&op).unwrap());

        let fetched = db.get(&op).unwrap().unwrap();
        assert_eq!(fetched.value(), 1_000);
        assert_eq!(fetched.height, 10);

        let removed = db.remove(&op).unwrap();
        assert_eq!(removed.value(), 1_000);
        assert!(!db.contains(&op).unwrap());
    }

    #[test]
    fn remove_unknown_errors() {
        let db = UTXOSet::new(tmp_path()).unwrap();
        let result = db.remove(&make_outpoint(0));
        assert!(matches!(result, Err(UtxoError::UnknownUtxo { .. })));
    }

    #[test]
    fn insert_duplicate_errors() {
        let db = UTXOSet::new(tmp_path()).unwrap();
        let op = make_outpoint(0);
        let utxo = make_utxo(500, 1, false);
        db.insert(&op, &utxo).unwrap();
        let result = db.insert(&op, &utxo);
        assert!(matches!(result, Err(UtxoError::DuplicateUtxo { .. })));
    }

    // ── apply_block ───────────────────────────────────────────────────────────

    #[test]
    fn apply_coinbase_block_adds_utxos() {
        let db = UTXOSet::new(tmp_path()).unwrap();
        let block = make_coinbase_block(0);
        let undo = db.apply_block(&block).unwrap();

        // Coinbase tx has one output — it should now be in the UTXO set.
        assert_eq!(db.len().unwrap(), 1);
        assert_eq!(db.total_supply().unwrap(), 50_0000_0000);

        // No UTXOs were spent (coinbase has no inputs consuming prior UTXOs).
        assert!(undo.spent_outputs.is_empty());
    }

    #[test]
    fn apply_block_consumes_and_creates_utxos() {
        use crate::{BlockHeader, TxInput, compute_merkle_root};

        let db = UTXOSet::new(tmp_path()).unwrap();

        // ── Seed: apply block 0 to create a spendable UTXO ────────────────
        let block0 = make_coinbase_block(0);
        db.apply_block(&block0).unwrap();
        assert_eq!(db.len().unwrap(), 1);

        // Recover the coinbase TXID so we can reference it in block 1.
        let cb_txid = block0.transactions[0].compute_hash().unwrap();
        let cb_outpoint = OutPoint { txid: cb_txid, vout: 0 };

        // ── Build block 1: spend the coinbase, create two new outputs ──────
        let spending_tx = Transaction {
            version: 1,
            inputs: vec![TxInput::new(cb_outpoint.clone(), dummy_script(), u32::MAX)],
            outputs: vec![make_output(20_0000_0000), make_output(29_9900_0000)],
            locktime: 0,
        };

        let cb1_data = 1u64.to_le_bytes().to_vec();
        let coinbase1 = Transaction {
            version: 1,
            inputs: vec![TxInput::new_coinbase(cb1_data).unwrap()],
            outputs: vec![make_output(50_0000_0000)],
            locktime: 0,
        };

        let txs1 = vec![coinbase1, spending_tx];
        let merkle_root1 = compute_merkle_root(&txs1).unwrap();

        let block1 = Block {
            header: BlockHeader {
                version: 1,
                previous_block_hash: block0.compute_hash().unwrap(),
                merkle_root: merkle_root1,
                timestamp: 1_700_000_001,
                difficulty_target: 0x1d00_ffff,
                nonce: 0,
                height: 1,
                uncle_hashes: vec![],
            },
            transactions: txs1,
            uncle_blocks: vec![],
        };

        let undo1 = db.apply_block(&block1).unwrap();

        // Block 0 coinbase consumed + 3 new outputs = 3 UTXOs total.
        assert_eq!(db.len().unwrap(), 3);

        // One UTXO was spent.
        assert_eq!(undo1.spent_outputs.len(), 1);
        assert_eq!(undo1.spent_outputs[0].outpoint, cb_outpoint);
        assert_eq!(undo1.spent_outputs[0].utxo.value(), 50_0000_0000);
    }

    // ── revert_block ──────────────────────────────────────────────────────────

    #[test]
    fn apply_then_revert_restores_state() {
        let db = UTXOSet::new(tmp_path()).unwrap();

        // Apply block 0 — creates 1 UTXO.
        let block0 = make_coinbase_block(0);
        let undo0 = db.apply_block(&block0).unwrap();
        assert_eq!(db.len().unwrap(), 1);

        // Revert block 0 — should return to empty.
        db.revert_block(&block0, &undo0).unwrap();
        assert_eq!(db.len().unwrap(), 0);
        assert!(db.is_empty().unwrap());
    }

    #[test]
    fn revert_mismatch_errors() {
        let db = UTXOSet::new(tmp_path()).unwrap();
        let block = make_coinbase_block(0);
        db.apply_block(&block).unwrap();

        // Tamper with the undo block: add a phantom spent output.
        let bad_undo = UndoBlock {
            block_hash: Hash::ZERO,
            spent_outputs: vec![SpentOutput {
                outpoint: make_outpoint(99),
                utxo: make_utxo(1, 0, false),
            }],
        };

        // Should fail — undo has 1 spent output but block has 0 spending inputs.
        let result = db.revert_block(&block, &bad_undo);
        assert!(matches!(result, Err(UtxoError::UndoMismatch)));
    }

    // ── Serialisation round-trip ──────────────────────────────────────────────

    #[test]
    fn utxo_serialise_roundtrip() {
        let utxo = make_utxo(42_0000_0000, 777, true);
        let bytes = serialise_utxo(&utxo).unwrap();
        let recovered = deserialise_utxo(&bytes).unwrap();
        assert_eq!(utxo, recovered);
    }

    #[test]
    fn undo_block_serde_roundtrip() {
        let undo = UndoBlock {
            block_hash: Hash::from_bytes([0xde; 32]),
            spent_outputs: vec![SpentOutput {
                outpoint: make_outpoint(0),
                utxo: make_utxo(1000, 5, false),
            }],
        };
        let json = serde_json::to_string(&undo).unwrap();
        let recovered: UndoBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(undo.block_hash, recovered.block_hash);
        assert_eq!(undo.spent_outputs.len(), recovered.spent_outputs.len());
    }
}

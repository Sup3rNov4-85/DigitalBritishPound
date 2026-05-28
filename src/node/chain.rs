use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::{
    consensus::next_difficulty_target,
    node::{
        mempool::Mempool,
        validation::{validate_block, validate_transaction, verify_block_pow, ValidationError},
    },
    storage::chaindb::{ChainDb, ChainDbError, ChainTip},
    types::utxo::{UTXOSet, UtxoError},
    Block, BlockHeader, Hash,
};

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("chain db: {0}")]
    Db(#[from] ChainDbError),
    #[error("utxo: {0}")]
    Utxo(#[from] UtxoError),
    #[error("validation: {0}")]
    Validation(#[from] ValidationError),
    #[error("serialization: {0}")]
    Ser(String),
    #[error("orphan block (unknown parent)")]
    Orphan,
    #[error("invalid height: expected {expected}, got {got}")]
    BadHeight { expected: u64, got: u64 },
    #[error("genesis must be height 0")]
    BadGenesis,
    #[error("mempool rejected tx")]
    MempoolRejected,
}

#[derive(Clone)]
pub struct Chain {
    inner: Arc<ChainInner>,
}

struct ChainInner {
    db: ChainDb,
    utxos: UTXOSet,
    mempool: Mutex<Mempool>,
    /// Blocks whose parent is known but is not the current tip.
    orphans: Mutex<HashMap<Hash, Block>>,
}

impl Chain {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, ChainError> {
        let data_dir = data_dir.as_ref();
        let db = ChainDb::open(data_dir.join("chain"))?;
        let utxos = UTXOSet::new(data_dir.join("utxo"))?;
        Ok(Self {
            inner: Arc::new(ChainInner {
                db,
                utxos,
                mempool: Mutex::new(Mempool::new()),
                orphans: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn tip(&self) -> Result<Option<ChainTip>, ChainError> {
        Ok(self.inner.db.tip()?)
    }

    pub fn db(&self) -> &ChainDb {
        &self.inner.db
    }

    pub fn utxos(&self) -> &UTXOSet {
        &self.inner.utxos
    }

    pub fn difficulty_for_next_block(&self) -> Result<u32, ChainError> {
        let height = match self.tip()? {
            Some(t) => t.height + 1,
            None => 0,
        };
        Ok(next_difficulty_target(&self.inner.db, height)?)
    }

    /// Select up to 2 uncle headers from the orphan pool for mining.
    pub fn select_uncles(&self) -> Result<Vec<BlockHeader>, ChainError> {
        let tip_h = self.tip()?.map(|t| t.height).unwrap_or(0);
        let next_h = tip_h + 1;
        let min_h = next_h.saturating_sub(crate::consensus::UNCLE_LOOKBACK_BLOCKS);
        let orphans = self.inner.orphans.lock().unwrap();
        let mut out = Vec::new();
        for block in orphans.values() {
            if block.header.height >= min_h && block.header.height < next_h {
                out.push(block.header.clone());
                if out.len() >= crate::consensus::MAX_UNCLES_PER_BLOCK {
                    break;
                }
            }
        }
        Ok(out)
    }

    pub fn accept_block(&self, block: &Block) -> Result<Option<Hash>, ChainError> {
        verify_block_pow(&block.header)?;

        let hash = block
            .compute_hash()
            .map_err(|e| ChainError::Ser(e.to_string()))?;

        if self.inner.db.get_block(&hash).is_ok() {
            return Ok(None);
        }

        match self.tip()? {
            None => {
                if block.header.height != 0 || block.header.previous_block_hash != Hash::ZERO {
                    return self.try_orphan(block);
                }
                // Consensus lock: only accept the one true genesis.
                let expected = crate::consensus::expected_genesis_hash();
                if hash != expected {
                    return Err(ChainError::Ser(format!(
                        "wrong genesis hash: got={} expected={}",
                        hash.to_hex(),
                        expected.to_hex()
                    )));
                }
            }
            Some(tip) => {
                if block.header.previous_block_hash != tip.hash {
                    return self.try_orphan(block);
                }
                if block.header.height != tip.height + 1 {
                    return Err(ChainError::BadHeight {
                        expected: tip.height + 1,
                        got: block.header.height,
                    });
                }
            }
        }

        self.apply_block(block)?;
        self.process_orphans()?;
        Ok(Some(hash))
    }

    fn try_orphan(&self, block: &Block) -> Result<Option<Hash>, ChainError> {
        // Parent must exist on chain (but not be tip) to store as orphan candidate / uncle.
        if self
            .inner
            .db
            .get_block(&block.header.previous_block_hash)
            .is_err()
        {
            return Err(ChainError::Orphan);
        }
        verify_block_pow(&block.header)?;
        let hash = block
            .compute_hash()
            .map_err(|e| ChainError::Ser(e.to_string()))?;
        self.inner.orphans.lock().unwrap().insert(hash, block.clone());
        Ok(None)
    }

    fn apply_block(&self, block: &Block) -> Result<(), ChainError> {
        validate_block(&self.inner.db, &self.inner.utxos, block)?;
        self.inner.utxos.apply_block(block)?;
        self.inner.db.put_block(block)?;

        let confirmed: Vec<[u8; 32]> = block
            .transactions
            .iter()
            .skip(1)
            .filter_map(|tx| tx.compute_hash().ok().map(|h| *h.as_bytes()))
            .collect();
        self.inner.mempool.lock().unwrap().remove_confirmed(&confirmed);

        // Remove from orphan pool if it was there.
        if let Ok(h) = block.compute_hash() {
            self.inner.orphans.lock().unwrap().remove(&h);
        }
        Ok(())
    }

    fn process_orphans(&self) -> Result<(), ChainError> {
        loop {
            let tip = match self.tip()? {
                Some(t) => t,
                None => break,
            };
            let candidate = {
                let orphans = self.inner.orphans.lock().unwrap();
                orphans
                    .values()
                    .find(|b| b.header.previous_block_hash == tip.hash && b.header.height == tip.height + 1)
                    .cloned()
            };
            match candidate {
                Some(b) => {
                    self.apply_block(&b)?;
                }
                None => break,
            }
        }
        Ok(())
    }

    pub fn add_mempool_tx(&self, tx: crate::Transaction) -> Result<(), ChainError> {
        let height = self.tip()?.map(|t| t.height).unwrap_or(0);
        let info = validate_transaction(&self.inner.utxos, &tx, height)?;
        let mut mp = self.inner.mempool.lock().unwrap();
        if !mp.insert(tx, info.fees_units) {
            return Err(ChainError::MempoolRejected);
        }
        Ok(())
    }

    pub fn mempool_snapshot(&self) -> Vec<crate::Transaction> {
        self.inner.mempool.lock().unwrap().snapshot()
    }

    pub fn mempool_fees(&self) -> Result<u64, ChainError> {
        let height = self.tip()?.map(|t| t.height).unwrap_or(0);
        let mut total = 0u64;
        for tx in self.mempool_snapshot() {
            let info = validate_transaction(&self.inner.utxos, &tx, height)?;
            total = total.saturating_add(info.fees_units);
        }
        Ok(total)
    }
}

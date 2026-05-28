use std::path::Path;

use rocksdb::{Options, DB};
use serde::{Deserialize, Serialize};

use crate::{Block, Hash};

#[derive(Debug, thiserror::Error)]
pub enum ChainDbError {
    #[error("rocksdb error: {0}")]
    RocksDb(#[from] rocksdb::Error),
    #[error("serialization error: {0}")]
    Ser(String),
    #[error("block not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainTip {
    pub height: u64,
    pub hash: Hash,
}

/// Very small RocksDB-backed chain store.
///
/// Keyspace (single CF):
/// - `b"tip"` -> bincode(ChainTip)
/// - `b"b:" + <hash32>` -> bincode(Block)
/// - `b"h:" + <height_be_u64>` -> <hash32>
pub struct ChainDb {
    db: DB,
}

impl ChainDb {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, ChainDbError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path)?;
        Ok(Self { db })
    }

    pub fn tip(&self) -> Result<Option<ChainTip>, ChainDbError> {
        match self.db.get(b"tip")? {
            Some(v) => Ok(Some(
                bincode::deserialize(&v).map_err(|e| ChainDbError::Ser(e.to_string()))?,
            )),
            None => Ok(None),
        }
    }

    pub fn get_block(&self, hash: &Hash) -> Result<Block, ChainDbError> {
        let key = block_key(hash);
        let v = self
            .db
            .get(key)?
            .ok_or_else(|| ChainDbError::NotFound(hash.to_hex()))?;
        bincode::deserialize(&v).map_err(|e| ChainDbError::Ser(e.to_string()))
    }

    pub fn get_block_at_height(&self, height: u64) -> Result<Option<Block>, ChainDbError> {
        match self.get_hash_by_height(height)? {
            Some(h) => Ok(Some(self.get_block(&h)?)),
            None => Ok(None),
        }
    }

    pub fn get_hash_by_height(&self, height: u64) -> Result<Option<Hash>, ChainDbError> {
        let key = height_key(height);
        match self.db.get(key)? {
            Some(v) => {
                if v.len() != 32 {
                    return Err(ChainDbError::Ser("bad hash length at height".into()));
                }
                let mut b = [0u8; 32];
                b.copy_from_slice(&v);
                Ok(Some(Hash::from_bytes(b)))
            }
            None => Ok(None),
        }
    }

    pub fn put_block(&self, block: &Block) -> Result<Hash, ChainDbError> {
        let hash = block.compute_hash().map_err(|e| ChainDbError::Ser(e.to_string()))?;
        let height = block.header.height;

        let bbytes = bincode::serialize(block).map_err(|e| ChainDbError::Ser(e.to_string()))?;
        self.db.put(block_key(&hash), bbytes)?;
        self.db.put(height_key(height), hash.as_bytes())?;
        let tip = ChainTip { height, hash };
        let tip_bytes = bincode::serialize(&tip).map_err(|e| ChainDbError::Ser(e.to_string()))?;
        self.db.put(b"tip", tip_bytes)?;
        Ok(hash)
    }
}

fn block_key(hash: &Hash) -> Vec<u8> {
    let mut k = Vec::with_capacity(2 + 32);
    k.extend_from_slice(b"b:");
    k.extend_from_slice(hash.as_bytes());
    k
}

fn height_key(height: u64) -> [u8; 10] {
    let mut k = [0u8; 10];
    k[..2].copy_from_slice(b"h:");
    k[2..].copy_from_slice(&height.to_be_bytes());
    k
}


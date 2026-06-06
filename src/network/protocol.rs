use serde::{Deserialize, Serialize};

use crate::{Block, Transaction};

/// Gossip + request/response payloads (bincode-encoded).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkMessage {
    Block(Block),
    Tx(Transaction),
    /// Request a block by height (for catch-up sync).
    GetBlock { height: u64 },
    BlockReply { height: u64, block: Option<Block> },
    /// Encrypted peer registry sync — merge into local peers.enc.
    PeerList { peers: Vec<String> },
}

pub const TOPIC_BLOCKS: &str = "dbc/blocks/v1";
pub const TOPIC_TXS: &str = "dbc/txs/v1";
pub const TOPIC_SYNC: &str = "dbc/sync/v1";
pub const TOPIC_PEERS: &str = "dbc/peers/v1";

pub fn encode(msg: &NetworkMessage) -> Result<Vec<u8>, bincode::Error> {
    bincode::serialize(msg)
}

pub fn decode(bytes: &[u8]) -> Result<NetworkMessage, bincode::Error> {
    bincode::deserialize(bytes)
}

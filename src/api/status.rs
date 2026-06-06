//! Local status snapshot written by the running node for the UI (and optional HTTP API).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeStatusSnapshot {
    pub tip_height: Option<u64>,
    pub peer_count: u32,
    pub mining_enabled: bool,
    pub listening: bool,
    /// Reachable nodes in encrypted peers.enc (community pool size).
    #[serde(default)]
    pub peer_pool_size: u32,
    /// `off` | `solo` | `lead` | `sync` — how this node is helping the chain.
    #[serde(default)]
    pub mining_mode: String,
}

pub fn status_path(data_dir: &Path) -> PathBuf {
    data_dir.join("status.json")
}

pub fn write_status(data_dir: &Path, snap: &NodeStatusSnapshot) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let json = serde_json::to_vec_pretty(snap)?;
    std::fs::write(status_path(data_dir), json)
}

pub fn read_status(data_dir: &Path) -> Option<NodeStatusSnapshot> {
    let bytes = std::fs::read(status_path(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

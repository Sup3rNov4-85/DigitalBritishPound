use std::path::Path;

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

/// Official bootstrap — DNS + port only (no `/p2p/` — wrong peer IDs break dials even when the port is open).
pub const DUCKDNS_BOOTSTRAP: &str = "/dns4/digitalbritishpound.duckdns.org/tcp/8333";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerEntry {
    pub multiaddr: String,
    pub added_unix: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PeerRegistry {
    entries: Vec<PeerEntry>,
}

impl PeerRegistry {
    pub fn default_bundled() -> Self {
        let mut reg = Self::default();
        reg.add_multiaddr_str(DUCKDNS_BOOTSTRAP);
        reg
    }

    pub fn load_or_create(data_path: &Path, bundled_path: &Path) -> anyhow::Result<Self> {
        if data_path.exists() {
            return Self::load_encrypted(data_path);
        }
        if bundled_path.exists() {
            if let Ok(reg) = Self::load_encrypted(bundled_path) {
                reg.save_encrypted(data_path)?;
                return Ok(reg);
            }
        }
        let reg = Self::default_bundled();
        reg.save_encrypted(data_path)?;
        Ok(reg)
    }

    pub fn load_encrypted(path: &Path) -> anyhow::Result<Self> {
        let blob = std::fs::read(path)?;
        let entries: Vec<PeerEntry> = decrypt_json(&blob)?;
        Ok(Self { entries })
    }

    pub fn save_encrypted(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let blob = encrypt_json(&self.entries)?;
        std::fs::write(path, blob)?;
        Ok(())
    }

    pub fn add_multiaddr_str(&mut self, addr: &str) -> bool {
        let addr = addr.trim();
        if addr.is_empty() || is_unusable_dial_addr(addr) {
            return false;
        }
        if self.entries.iter().any(|e| e.multiaddr == addr) {
            return false;
        }
        self.entries.push(PeerEntry {
            multiaddr: addr.to_string(),
            added_unix: now_unix(),
        });
        true
    }

    /// Merge remote pool entries — keep the oldest `added_unix` per address (first joiners stay first).
    pub fn merge_peer_entries(&mut self, peers: &[PeerEntry]) -> usize {
        let mut added = 0;
        for p in peers {
            if is_unusable_dial_addr(&p.multiaddr) {
                continue;
            }
            if let Some(existing) = self
                .entries
                .iter_mut()
                .find(|e| e.multiaddr == p.multiaddr)
            {
                if p.added_unix < existing.added_unix {
                    existing.added_unix = p.added_unix;
                }
                continue;
            }
            self.entries.push(p.clone());
            added += 1;
        }
        added
    }

    pub fn merge_peer_strings(&mut self, peers: &[String]) -> usize {
        let entries: Vec<PeerEntry> = peers
            .iter()
            .filter(|s| !is_unusable_dial_addr(s))
            .map(|s| PeerEntry {
                multiaddr: s.clone(),
                added_unix: now_unix(),
            })
            .collect();
        self.merge_peer_entries(&entries)
    }

    pub fn add_multiaddr(&mut self, addr: &Multiaddr) -> bool {
        self.add_multiaddr_str(&addr.to_string())
    }

    /// Register this node when we know a dialable public multiaddr (never 0.0.0.0 listen).
    pub fn ensure_self(&mut self, listen: &Multiaddr, peer_id: &PeerId) -> bool {
        if is_unspec_listen(listen) {
            return false;
        }
        let with_peer = listen.clone().with_p2p(*peer_id).unwrap_or_else(|_| listen.clone());
        self.add_multiaddr(&with_peer)
    }

    /// Record a peer we successfully reached (grows the encrypted bootstrap pool).
    pub fn record_reachable_peer(&mut self, addr: &Multiaddr) -> bool {
        self.add_multiaddr(addr)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn community_len(&self) -> usize {
        self.entries.iter().filter(|e| !is_duckdns(&e.multiaddr)).count()
    }

    pub fn entries(&self) -> &[PeerEntry] {
        &self.entries
    }

    /// Replace stale DuckDNS entries (e.g. old shipped `/p2p/` with wrong peer id).
    pub fn refresh_duckdns_bootstrap(&mut self) -> bool {
        let had_stale = self.entries.iter().any(|e| {
            is_duckdns(&e.multiaddr) && e.multiaddr != DUCKDNS_BOOTSTRAP
        });
        self.entries.retain(|e| !is_duckdns(&e.multiaddr));
        let added = self.add_multiaddr_str(DUCKDNS_BOOTSTRAP);
        had_stale || added
    }

    pub fn peer_strings(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.multiaddr.clone()).collect()
    }

    /// Dial order: community nodes oldest-first (first joiners in the pool), DuckDNS fallback last.
    pub fn dial_order(&self) -> Vec<Multiaddr> {
        let mut community: Vec<&PeerEntry> = self
            .entries
            .iter()
            .filter(|e| !is_duckdns(&e.multiaddr) && !is_unusable_dial_addr(&e.multiaddr))
            .collect();
        community.sort_by_key(|e| e.added_unix);

        let mut out = Vec::new();
        for e in community {
            if let Ok(addr) = e.multiaddr.parse::<Multiaddr>() {
                out.push(addr);
            }
        }

        let has_duck = self.entries.iter().any(|e| is_duckdns(&e.multiaddr));
        if has_duck {
            if let Ok(a) = DUCKDNS_BOOTSTRAP.parse() {
                out.push(a);
            }
        } else if out.is_empty() {
            if let Ok(a) = DUCKDNS_BOOTSTRAP.parse() {
                out.push(a);
            }
        }
        out
    }
}

fn is_duckdns(s: &str) -> bool {
    s.contains("digitalbritishpound.duckdns.org")
}

fn is_unusable_dial_addr(s: &str) -> bool {
    s.contains("/ip4/0.0.0.0/")
        || s.contains("/ip4/127.0.0.1/")
        || s.contains("/ip6/::/")
        || s.contains("/ip6/::1/")
}

fn is_unspec_listen(addr: &Multiaddr) -> bool {
    use libp2p::multiaddr::Protocol;
    use std::net::{Ipv4Addr, Ipv6Addr};
    addr.iter().any(|p| match p {
        Protocol::Ip4(ip) => ip == Ipv4Addr::UNSPECIFIED || ip == Ipv4Addr::LOCALHOST,
        Protocol::Ip6(ip) => ip == Ipv6Addr::UNSPECIFIED || ip == Ipv6Addr::LOCALHOST,
        _ => false,
    })
}

fn registry_key() -> [u8; 32] {
    *blake3::hash(
        format!(
            "dbc-peer-registry-v1:{}",
            crate::consensus::GENESIS_HASH_HEX
        )
        .as_bytes(),
    )
    .as_bytes()
}

fn encrypt_json(entries: &[PeerEntry]) -> anyhow::Result<Vec<u8>> {
    let plain = serde_json::to_vec(entries)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&registry_key())?;
    let nonce_bytes: [u8; 12] = rand::random();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut out = nonce_bytes.to_vec();
    out.extend(
        cipher
            .encrypt(nonce, plain.as_ref())
            .map_err(|e| anyhow::anyhow!("encrypt peers: {e}"))?,
    );
    Ok(out)
}

fn decrypt_json(blob: &[u8]) -> anyhow::Result<Vec<PeerEntry>> {
    anyhow::ensure!(blob.len() > 12, "peers.enc too short");
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = ChaCha20Poly1305::new_from_slice(&registry_key())?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ct)
        .map_err(|e| anyhow::anyhow!("decrypt peers (wrong file or corrupt): {e}"))?;
    Ok(serde_json::from_slice(&plain)?)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_nodes_dial_before_duckdns_fallback() {
        let mut reg = PeerRegistry::default_bundled();
        reg.add_multiaddr_str(
            "/ip4/1.2.3.4/tcp/8333/p2p/12D3KooWH3VQcmaw7b9wEBYYjVDr6aovGgg75RkkBLyZZbFRbhxy",
        );
        let order = reg.dial_order();
        assert_eq!(order.len(), 2);
        assert!(!is_duckdns(&order[0].to_string()));
        assert!(is_duckdns(&order[1].to_string()));
    }

    #[test]
    fn skips_unusable_listen_addresses() {
        let mut reg = PeerRegistry::default();
        assert!(!reg.add_multiaddr_str("/ip4/0.0.0.0/tcp/8333/p2p/12D3KooWExamplePeerIdExamplePeerIdExampl"));
    }

    #[test]
    fn merge_keeps_oldest_timestamp() {
        let mut reg = PeerRegistry::default();
        reg.merge_peer_entries(&[PeerEntry {
            multiaddr: "/ip4/1.1.1.1/tcp/8333/p2p/12D3KooWExamplePeerIdExamplePeerIdExampl".into(),
            added_unix: 50,
        }]);
        reg.merge_peer_entries(&[PeerEntry {
            multiaddr: "/ip4/1.1.1.1/tcp/8333/p2p/12D3KooWExamplePeerIdExamplePeerIdExampl".into(),
            added_unix: 999,
        }]);
        assert_eq!(reg.entries[0].added_unix, 50);
    }

    #[test]
    fn duckdns_fallback_when_pool_empty() {
        let reg = PeerRegistry::default_bundled();
        let order = reg.dial_order();
        assert_eq!(order.len(), 1);
        assert!(is_duckdns(&order[0].to_string()));
    }

    #[test]
    fn refresh_duckdns_replaces_stale_p2p() {
        let mut reg = PeerRegistry::default_bundled();
        reg.add_multiaddr_str(
            "/dns4/digitalbritishpound.duckdns.org/tcp/8333/p2p/12D3KooWStalePeerIdStalePeerIdStale",
        );
        assert!(reg.refresh_duckdns_bootstrap());
        assert!(reg.peer_strings().iter().any(|s| s == DUCKDNS_BOOTSTRAP));
        assert!(!reg
            .peer_strings()
            .iter()
            .any(|s| s.contains("StalePeerId")));
    }

    #[test]
    fn encrypt_roundtrip() {
        let reg = PeerRegistry::default_bundled();
        let blob = encrypt_json(&reg.entries).unwrap();
        let back = decrypt_json(&blob).unwrap();
        assert_eq!(back.len(), 1);
    }
}

use std::path::Path;

use libp2p::Multiaddr;

/// Load peer multiaddrs from a text file (one per line, `#` comments allowed).
pub fn load_peers_file(path: &Path) -> anyhow::Result<Vec<Multiaddr>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        match line.parse::<Multiaddr>() {
            Ok(a) => out.push(a),
            Err(e) => tracing::warn!("skip invalid peer line '{line}': {e}"),
        }
    }
    Ok(out)
}

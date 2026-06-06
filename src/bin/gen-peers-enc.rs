//! Generate release-package/peers.enc (encrypted peer pool with DuckDNS bootstrap).
//!
//! DuckDNS is stored DNS-only (no `/p2p/` suffix) so dials work even when the
//! answering peer id differs from an old shipped id. Community nodes with full
//! multiaddrs are learned at runtime and merged into the pool.

use std::path::PathBuf;

use clap::Parser;
use libp2p::identity::Keypair;

#[derive(Parser)]
struct Args {
    /// Optional data dir (reads `peer_key` — prints peer id for release notes only).
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("release-package").join("peers.enc");

    let reg = dbc_node::network::peer_registry::PeerRegistry::default_bundled();

    if let Some(data_dir) = args.data_dir {
        let key_path = data_dir.join("peer_key");
        if key_path.exists() {
            let bytes = std::fs::read(&key_path)?;
            let kp = Keypair::from_protobuf_encoding(&bytes)?;
            let peer_id = kp.public().to_peer_id();
            println!("release operator peer id (for docs / DuckDNS seed): {peer_id}");
        } else {
            eprintln!("warning: no peer_key in {} — run the node once first", data_dir.display());
        }
    }

    reg.save_encrypted(&out)?;
    println!("Wrote {} ({} peers)", out.display(), reg.peer_strings().len());
    for p in reg.peer_strings() {
        println!("  {p}");
    }
    Ok(())
}

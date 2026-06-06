//! Generate release-package/peers.enc (encrypted peer list with DuckDNS bootstrap).
//!
//! Optional: pass `--data-dir` (founder's `./data`) to also embed their `/p2p/` address
//! for Kademlia after the DNS bootstrap line.

use std::path::PathBuf;

use clap::Parser;
use libp2p::identity::Keypair;

#[derive(Parser)]
struct Args {
    /// Founder's data dir (reads `peer_key` for optional full multiaddr).
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("release-package").join("peers.enc");

    let mut reg = dbc_node::network::peer_registry::PeerRegistry::default_bundled();

    if let Some(data_dir) = args.data_dir {
        let key_path = data_dir.join("peer_key");
        if key_path.exists() {
            let bytes = std::fs::read(&key_path)?;
            let kp = Keypair::from_protobuf_encoding(&bytes)?;
            let peer_id = kp.public().to_peer_id();
            let full = format!(
                "{}/p2p/{}",
                dbc_node::network::peer_registry::DUCKDNS_BOOTSTRAP, peer_id
            );
            reg.add_multiaddr_str(&full);
            println!("founder peer id: {peer_id}");
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

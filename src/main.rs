use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use libp2p::Multiaddr;
use tracing_subscriber::EnvFilter;

use dbc_node::{
    crypto::wallet::{Address, Wallet},
    network::{run_p2p, P2pConfig},
    node::{
        chain::Chain,
        genesis::{export_genesis_json, import_genesis_json, mine_genesis, GENESIS_MESSAGE},
        miner::Miner,
        validation::{build_p2addr_script_pubkey, build_script_sig, sighash},
    },
    OutPoint, Script, Transaction, TxInput, TxOutput,
    Hash,
};

#[derive(Parser)]
#[command(name = "dbc-node", version, about = "Digital British Coin node (whitepaper v1.0)")]
struct Cli {
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    WalletNew,
    WalletAddr { mnemonic: String },
    /// Initialize genesis block (height 0) if chain is empty.
    Init {
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        timestamp: Option<u32>,
        /// Safety catch: prevents accidental creation of a new chain.
        /// You must pass this to run `init`.
        #[arg(long)]
        force_new_chain: bool,
    },
    /// Export genesis block JSON + hash after `init` (no network; safe for anonymous launch posts).
    ExportGenesis {
        #[arg(long, default_value = "genesis.json")]
        out: PathBuf,
    },
    /// Import the published genesis block (height 0) if the local chain is empty.
    ImportGenesis {
        #[arg(long, default_value = "genesis.json")]
        genesis: PathBuf,
    },
    /// Print libp2p peer id from `data/peer_key` (community seeds only — skip if staying anonymous).
    PeerId,
    Run {
        #[arg(long, default_value = "/ip4/0.0.0.0/tcp/8333")]
        listen: String,
        #[arg(long = "bootstrap")]
        bootstrap: Vec<String>,
        #[arg(long, default_value = "config/peers.txt")]
        peers_file: PathBuf,
        #[arg(long)]
        mine: bool,
        /// Optional file-based mining control (for the Windows UI).
        /// If provided, the node mines only when this file contains "1".
        #[arg(long)]
        mine_ctl_file: Option<PathBuf>,
        #[arg(long)]
        address: Option<String>,
        /// Kademlia DHT for decentralised discovery (default on).
        #[arg(long)]
        no_dht: bool,
        /// LAN mDNS (off by default — do not use on home networks for anonymous launch).
        #[arg(long)]
        mdns: bool,
        /// Shipped encrypted peer list (default: peers.enc next to cwd).
        #[arg(long)]
        bundled_peers: Option<PathBuf>,
        /// Advanced: listen only, do not dial peers (not used by the Windows app).
        #[arg(long)]
        host_only: bool,
    },
    /// Localhost read-only HTTP API (GET /status, /balance). Stop the node first.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8334")]
        listen: String,
    },
    Mine {
        #[arg(long, default_value_t = 1)]
        blocks: u64,
        #[arg(long)]
        address: Option<String>,
    },
    Send {
        #[arg(long, required_unless_present = "from_mnemonic_file")]
        from_mnemonic: Option<String>,
        #[arg(long, required_unless_present = "from_mnemonic")]
        from_mnemonic_file: Option<PathBuf>,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount_dbc: u64,
        #[arg(long, default_value_t = 0)]
        fee_dbc: u64,
    },
    /// Recent credits to an address (coinbase + received outputs).
    History {
        #[arg(long)]
        address: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Show confirmed balance for an address from the local UTXO set.
    Balance {
        /// Address to query (dbc1...)
        #[arg(long)]
        address: String,
        /// Include immature coinbase outputs in the total.
        #[arg(long)]
        include_immature: bool,
    },
    Info,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("dbc_node=info".parse()?))
        .init();

    let cli = Cli::parse();
    let chain = Chain::open(&cli.data_dir)?;

    match cli.cmd {
        Command::WalletNew => {
            let m = Wallet::generate_mnemonic();
            let w = Wallet::from_mnemonic(&m)?;
            println!("mnemonic: {}", m.to_string());
            println!("address: {}", w.address().to_bech32m()?);
        }
        Command::WalletAddr { mnemonic } => {
            let m = bip39::Mnemonic::parse(mnemonic)?;
            let w = Wallet::from_mnemonic(&m)?;
            println!("{}", w.address().to_bech32m()?);
        }
        Command::Init {
            address,
            timestamp,
            force_new_chain,
        } => {
            anyhow::ensure!(
                force_new_chain,
                "refusing to run `init` (would create a new chain). Use the published genesis, or pass --force-new-chain if you REALLY intend to start a new chain."
            );
            if chain.tip()?.is_some() {
                anyhow::bail!("chain already initialized");
            }
            let payout = resolve_payout(address)?;
            let ts = timestamp.unwrap_or_else(now_secs);
            println!("mining genesis (BritishWork)…");
            let block = mine_genesis(ts, payout).map_err(anyhow::Error::msg)?;
            let hash = chain
                .accept_block(&block)?
                .expect("genesis must be accepted");
            println!("genesis hash={}", hash.to_hex());
            println!("message: {GENESIS_MESSAGE}");
            println!("next: `export-genesis` to publish hash + genesis.json without running P2P");
        }
        Command::ExportGenesis { out } => {
            let block = chain
                .db()
                .get_block_at_height(0)
                .map_err(|e| anyhow::anyhow!(e))?
                .ok_or_else(|| anyhow::anyhow!("no genesis — run `init` first"))?;
            export_genesis_json(&block, &out).map_err(anyhow::Error::msg)?;
        }
        Command::ImportGenesis { genesis } => {
            if chain.tip()?.is_some() {
                println!("chain already has blocks — import skipped");
            } else {
                let block = import_genesis_json(&genesis).map_err(anyhow::Error::msg)?;
                let hash = chain
                    .accept_block(&block)?
                    .ok_or_else(|| anyhow::anyhow!("genesis already stored"))?;
                println!("imported genesis height=0 hash={}", hash.to_hex());
            }
        }
        Command::PeerId => {
            let key_path = cli.data_dir.join("peer_key");
            if !key_path.exists() {
                anyhow::bail!("no peer_key — run `run` once on a seed node or copy a generated key");
            }
            let bytes = std::fs::read(&key_path)?;
            let kp = libp2p::identity::Keypair::from_protobuf_encoding(&bytes)?;
            println!("{}", kp.public().to_peer_id());
        }
        Command::Run {
            listen,
            bootstrap,
            peers_file,
            mine,
            mine_ctl_file,
            address,
            no_dht,
            mdns,
            bundled_peers,
            host_only,
        } => {
            // If mining is controlled via a file (UI toggle), payout is optional at startup.
            let payout = if mine {
                Some(resolve_payout(address)?)
            } else if let Some(ref a) = address {
                if a.trim().is_empty() {
                    None
                } else {
                    Some(Address::from_bech32m(a)?)
                }
            } else {
                None
            };
            let listen: Multiaddr = listen.parse()?;
            let bootstrap: Result<Vec<_>, _> = bootstrap.iter().map(|s| s.parse()).collect();
            let peers_path = if peers_file.exists() {
                Some(peers_file)
            } else {
                None
            };
            run_p2p(
                &cli.data_dir,
                chain,
                P2pConfig {
                    listen,
                    bootstrap: bootstrap?,
                    peers_file: peers_path,
                    peers_enc_path: cli.data_dir.join("peers.enc"),
                    bundled_peers_enc: bundled_peers,
                    dial_peers: !host_only,
                    mine,
                    mine_ctl_file,
                    payout,
                    enable_mdns: mdns,
                    enable_dht: !no_dht,
                },
            )
            .await?;
        }
        Command::Mine { blocks, address } => {
            anyhow::ensure!(chain.tip()?.is_some(), "run `init` first to create genesis");
            let payout = resolve_payout(address)?;
            let mut prev_hash = chain.tip()?.map(|t| t.hash).unwrap_or(Hash::ZERO);
            let mut height = chain.tip()?.map(|t| t.height + 1).unwrap_or(0);
            let msg = GENESIS_MESSAGE.as_bytes();

            for _ in 0..blocks {
                let difficulty = chain.difficulty_for_next_block()?;
                let fees = chain.mempool_fees()?;
                let uncles = chain.select_uncles()?;
                let block = Miner::mine_next_block(
                    prev_hash,
                    height,
                    difficulty,
                    payout,
                    msg,
                    chain.mempool_snapshot(),
                    fees,
                    uncles,
                )?;
                let hash = chain
                    .accept_block(&block)?
                    .expect("mined block must be accepted");
                println!("mined height={} hash={}", height, hash.to_hex());
                prev_hash = hash;
                height += 1;
            }
        }
        Command::Send {
            from_mnemonic,
            from_mnemonic_file,
            to,
            amount_dbc,
            fee_dbc,
        } => {
            let mnemonic = if let Some(m) = from_mnemonic {
                m
            } else if let Some(path) = from_mnemonic_file {
                let m = std::fs::read_to_string(&path)?;
                let _ = std::fs::remove_file(&path);
                m
            } else {
                anyhow::bail!("provide --from-mnemonic or --from-mnemonic-file");
            };
            run_send(&chain, &mnemonic, &to, amount_dbc, fee_dbc)?;
        }
        Command::History { address, limit } => {
            anyhow::ensure!(chain.tip()?.is_some(), "chain is empty");
            let addr = Address::from_bech32m(&address)?;
            let entries = dbc_node::node::wallet_query::history_for_address(&chain, &addr, limit)?;
            if entries.is_empty() {
                println!("no history for {}", addr.to_bech32m()?);
            } else {
                for e in entries {
                    println!(
                        "height={} kind={} amount={} DBC",
                        e.height, e.kind, e.amount_dbc
                    );
                }
            }
        }
        Command::Serve { listen } => {
            let bind: std::net::SocketAddr = listen.parse()?;
            dbc_node::api::http::serve_local(bind, cli.data_dir.clone(), chain).await?;
        }
        Command::Balance {
            address,
            include_immature,
        } => {
            anyhow::ensure!(chain.tip()?.is_some(), "chain is empty — sync from a peer first");
            let addr = Address::from_bech32m(&address)?;
            let summary =
                dbc_node::node::wallet_query::balance_for_address(&chain, &addr, include_immature)?;
            println!(
                "{}",
                dbc_node::node::wallet_query::format_balance_display(&summary, include_immature)
            );
        }
        Command::Info => match chain.tip()? {
            Some(tip) => {
                println!("tip height={} hash={}", tip.height, tip.hash.to_hex());
                let diff = chain.difficulty_for_next_block()?;
                println!("next difficulty (compact)=0x{diff:08x}");
                println!(
                    "mempool txs={}",
                    chain.mempool_snapshot().len()
                );
                println!(
                    "britishwork memory: {} MiB",
                    dbc_node::crypto::british_work::memory_bytes() / (1024 * 1024)
                );
            }
            None => println!("chain is empty — run `import-genesis` or sync from a peer"),
        },
    }

    Ok(())
}

fn resolve_payout(address: Option<String>) -> anyhow::Result<Address> {
    if let Some(a) = address {
        return Ok(Address::from_bech32m(&a)?);
    }
    let m = Wallet::generate_mnemonic();
    let w = Wallet::from_mnemonic(&m)?;
    println!("generated mnemonic: {}", m.to_string());
    println!("mining address: {}", w.address().to_bech32m()?);
    Ok(w.address())
}

fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

fn run_send(
    chain: &Chain,
    from_mnemonic: &str,
    to: &str,
    amount_dbc: u64,
    fee_dbc: u64,
) -> anyhow::Result<()> {
    anyhow::ensure!(chain.tip()?.is_some(), "chain is empty — sync first");
    let from_m = bip39::Mnemonic::parse(from_mnemonic)?;
    let from_w = Wallet::from_mnemonic(&from_m)?;
    let to_addr = Address::from_bech32m(to)?;

    let amount = amount_dbc * dbc_node::consensus::UNITS_PER_DBC;
    let fee = fee_dbc * dbc_node::consensus::UNITS_PER_DBC;
    let need = amount + fee;
    let current_height = chain.tip()?.map(|t| t.height).unwrap_or(0);

    let mut selected: Vec<(OutPoint, dbc_node::types::utxo::Utxo)> = Vec::new();
    let mut total = 0u64;
    chain.utxos().for_each(|op, utxo| {
        if utxo.output.script_pubkey.as_bytes().len() == 21
            && utxo.output.script_pubkey.as_bytes()[0] == 0x14
            && &utxo.output.script_pubkey.as_bytes()[1..21] == from_w.address().as_bytes()
            && utxo.is_mature(current_height)
        {
            selected.push((op, utxo.clone()));
            total = total.saturating_add(utxo.value());
            if total >= need {
                return Ok(());
            }
        }
        Ok(())
    })?;
    anyhow::ensure!(total >= need, "insufficient funds: have={total} need={need}");

    let mut inputs = Vec::new();
    for (op, _) in &selected {
        inputs.push(TxInput::new(op.clone(), Script::new(vec![]), u32::MAX));
    }
    let mut outputs = vec![TxOutput::new(amount, build_p2addr_script_pubkey(to_addr))];
    let change = total - need;
    if change > 0 {
        outputs.push(TxOutput::new(
            change,
            build_p2addr_script_pubkey(from_w.address()),
        ));
    }
    let mut tx = Transaction {
        version: 1,
        inputs,
        outputs,
        locktime: 0,
    };
    let pubkey32 = from_w.pubkey32();
    for i in 0..tx.inputs.len() {
        let msg32 = sighash(&tx, i)?;
        let sig = from_w.sign(&msg32);
        tx.inputs[i].script_sig = build_script_sig(&sig, pubkey32);
    }
    chain.add_mempool_tx(tx)?;

    let prev_hash = chain.tip()?.map(|t| t.hash).unwrap_or(Hash::ZERO);
    let height = chain.tip()?.map(|t| t.height + 1).unwrap_or(0);
    let difficulty = chain.difficulty_for_next_block()?;
    let fees = chain.mempool_fees()?;
    let uncles = chain.select_uncles()?;
    let block = Miner::mine_next_block(
        prev_hash,
        height,
        difficulty,
        from_w.address(),
        GENESIS_MESSAGE.as_bytes(),
        chain.mempool_snapshot(),
        fees,
        uncles,
    )?;
    let hash = chain
        .accept_block(&block)?
        .expect("mined block must be accepted");
    println!("sent {} DBC to {} in block height={} hash={}", amount_dbc, to, height, hash.to_hex());
    Ok(())
}

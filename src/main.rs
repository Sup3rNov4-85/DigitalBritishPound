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
        genesis::{export_genesis_json, mine_genesis, GENESIS_MESSAGE},
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
    },
    Mine {
        #[arg(long, default_value_t = 1)]
        blocks: u64,
        #[arg(long)]
        address: Option<String>,
    },
    Send {
        #[arg(long)]
        from_mnemonic: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount_dbc: u64,
        #[arg(long, default_value_t = 0)]
        fee_dbc: u64,
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
        } => {
            // If mining is controlled via a file (UI toggle), we still need a payout address
            // so mining attempts can produce blocks.
            let payout = if mine || mine_ctl_file.is_some() {
                Some(resolve_payout(address)?)
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
            to,
            amount_dbc,
            fee_dbc,
        } => {
            anyhow::ensure!(chain.tip()?.is_some(), "run `init` first");
            let from_m = bip39::Mnemonic::parse(from_mnemonic)?;
            let from_w = Wallet::from_mnemonic(&from_m)?;
            let to_addr = Address::from_bech32m(&to)?;

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
            println!("mined payment in height={} hash={}", height, hash.to_hex());
        }
        Command::Balance {
            address,
            include_immature,
        } => {
            anyhow::ensure!(chain.tip()?.is_some(), "chain is empty — sync from a peer first");
            let addr = Address::from_bech32m(&address)?;
            let current_height = chain.tip()?.map(|t| t.height).unwrap_or(0);

            let mut total = 0u64;
            let mut spendable = 0u64;
            chain.utxos().for_each(|_op, utxo| {
                if utxo.output.script_pubkey.as_bytes().len() == 21
                    && utxo.output.script_pubkey.as_bytes()[0] == 0x14
                    && &utxo.output.script_pubkey.as_bytes()[1..21] == addr.as_bytes()
                {
                    total = total.saturating_add(utxo.value());
                    if utxo.is_mature(current_height) {
                        spendable = spendable.saturating_add(utxo.value());
                    }
                }
                Ok(())
            })?;

            let units_per = dbc_node::consensus::UNITS_PER_DBC;
            let shown = if include_immature { total } else { spendable };
            println!("address: {}", addr.to_bech32m()?);
            println!(
                "confirmed: {} pence ({}.{:08} DBC)",
                total,
                total / units_per,
                total % units_per
            );
            println!(
                "spendable: {} pence ({}.{:08} DBC){}",
                spendable,
                spendable / units_per,
                spendable % units_per,
                if include_immature {
                    ""
                } else {
                    "  (coinbase needs 100 blocks to mature)"
                }
            );
            if include_immature {
                println!(
                    "shown: {} pence ({}.{:08} DBC)  (--include-immature)",
                    shown,
                    shown / units_per,
                    shown % units_per
                );
            }
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
            None => println!("chain is empty — run `init` first"),
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

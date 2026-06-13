use std::path::Path;
use std::{fs, path::PathBuf};
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    gossipsub::{self, IdentTopic, MessageAuthenticity},
    identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    mdns,
    noise,
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour, SwarmEvent},
    tcp, upnp, yamux, Multiaddr, PeerId, StreamProtocol, SwarmBuilder,
};
use tracing::{info, warn};

use crate::{
    api::status::{write_status, NodeStatusSnapshot},
    crypto::wallet::Address,
    network::{
        peer_registry::PeerRegistry,
        protocol::{decode, encode, NetworkMessage, TOPIC_BLOCKS, TOPIC_PEERS, TOPIC_SYNC, TOPIC_TXS},
        seeds::load_peers_file,
    },
    node::{chain::Chain, miner::{Miner, MinerError}},
    Block, Hash,
};

const GENESIS_MSG: &[u8] =
    b"The Times 27/May/2026 \xE2\x80\x94 A nation overtaxed, searching for an alternative";

const KAD_PROTOCOL: &str = "/dbc/kad/1.0.0";

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "NodeEvent")]
struct NodeBehaviour {
    gossipsub: gossipsub::Behaviour,
    /// Kademlia DHT — decentralised peer discovery (whitepaper). No central seed required.
    kad: kad::Behaviour<MemoryStore>,
    identify: identify::Behaviour,
    /// UPnP / NAT-PMP automatic port mapping (on by default).
    upnp: Toggle<upnp::tokio::Behaviour>,
    /// LAN-only; off by default so your node does not advertise on local multicast.
    mdns: Toggle<mdns::tokio::Behaviour>,
}

#[derive(Debug)]
enum NodeEvent {
    Gossipsub(gossipsub::Event),
    Kad(kad::Event),
    Identify(identify::Event),
    Upnp(upnp::Event),
    Mdns(mdns::Event),
}

impl From<gossipsub::Event> for NodeEvent {
    fn from(e: gossipsub::Event) -> Self {
        NodeEvent::Gossipsub(e)
    }
}

impl From<kad::Event> for NodeEvent {
    fn from(e: kad::Event) -> Self {
        NodeEvent::Kad(e)
    }
}

impl From<identify::Event> for NodeEvent {
    fn from(e: identify::Event) -> Self {
        NodeEvent::Identify(e)
    }
}

impl From<upnp::Event> for NodeEvent {
    fn from(e: upnp::Event) -> Self {
        NodeEvent::Upnp(e)
    }
}

impl From<mdns::Event> for NodeEvent {
    fn from(e: mdns::Event) -> Self {
        NodeEvent::Mdns(e)
    }
}

pub struct P2pConfig {
    pub listen: Multiaddr,
    /// Extra bootstrap peers (CLI). Encrypted peers.enc is always used first (DuckDNS).
    pub bootstrap: Vec<Multiaddr>,
    pub peers_file: Option<std::path::PathBuf>,
    /// Local encrypted peer registry (`data/peers.enc`).
    pub peers_enc_path: PathBuf,
    /// Shipped `peers.enc` copied on first run (contains DuckDNS bootstrap).
    pub bundled_peers_enc: Option<PathBuf>,
    /// When false (listen-only), do not dial out — others find you via the peer pool.
    pub dial_peers: bool,
    pub mine: bool,
    /// Optional file-based mining control. When set, the node will mine only when this file contains "1".
    /// This lets a GUI toggle mining without restarting the node.
    pub mine_ctl_file: Option<PathBuf>,
    pub payout: Option<Address>,
    /// mDNS is for local dev only; keep false for anonymous public mining.
    pub enable_mdns: bool,
    pub enable_dht: bool,
    /// Ask the home router to open the listen port automatically (UPnP / NAT-PMP).
    pub enable_upnp: bool,
}

pub async fn run_p2p(data_dir: &Path, chain: Chain, cfg: P2pConfig) -> anyhow::Result<()> {
    let key_path = data_dir.join("peer_key");
    let (key, is_new_key) = load_or_create_key(&key_path)?;
    let local_peer_id = key.public().to_peer_id();

    let bundled = cfg
        .bundled_peers_enc
        .clone()
        .unwrap_or_else(|| PathBuf::from("peers.enc"));
    let mut registry =
        PeerRegistry::load_or_create(&cfg.peers_enc_path, &bundled)?;
    if registry.refresh_duckdns_bootstrap() {
        registry.save_encrypted(&cfg.peers_enc_path)?;
    }
    if registry.ensure_self(&cfg.listen, &local_peer_id) || is_new_key {
        registry.save_encrypted(&cfg.peers_enc_path)?;
        if is_new_key {
            info!("first run — joined the encrypted peer bootstrap pool");
        }
    }

    info!(
        "encrypted peer pool — {} bootstrap entries (community nodes dial first)",
        registry.len()
    );

    let blocks_topic = IdentTopic::new(TOPIC_BLOCKS);
    let txs_topic = IdentTopic::new(TOPIC_TXS);
    let sync_topic = IdentTopic::new(TOPIC_SYNC);
    let peers_topic = IdentTopic::new(TOPIC_PEERS);

    let extra_bootstrap = cfg.bootstrap.clone();
    let peers_file = cfg.peers_file.clone();
    let mut bootstrap = build_dial_targets(&registry, &extra_bootstrap, peers_file.as_deref());

    let enable_mdns = cfg.enable_mdns;
    let enable_dht = cfg.enable_dht;
    let enable_upnp = cfg.enable_upnp;
    let dial_peers = cfg.dial_peers;
    let peers_enc_path = cfg.peers_enc_path.clone();

    let mut swarm = SwarmBuilder::with_existing_identity(key.clone())
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|key| {
            let gs_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(1))
                .validation_mode(gossipsub::ValidationMode::Permissive)
                .build()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            let mut gs = gossipsub::Behaviour::new(MessageAuthenticity::Signed(key.clone()), gs_config)?;
            gs.subscribe(&blocks_topic)?;
            gs.subscribe(&txs_topic)?;
            gs.subscribe(&sync_topic)?;
            gs.subscribe(&peers_topic)?;

            let kad_config = kad::Config::new(StreamProtocol::new(KAD_PROTOCOL));
            let store = MemoryStore::new(key.public().to_peer_id());
            let mut kad_behaviour = kad::Behaviour::with_config(
                key.public().to_peer_id(),
                store,
                kad_config,
            );

            for addr in &bootstrap {
                if let Some(peer) = peer_id_from_multiaddr(addr) {
                    let dial_addr = addr.clone().with_p2p(peer).unwrap_or_else(|_| addr.clone());
                    kad_behaviour.add_address(&peer, dial_addr);
                }
            }

            let identify = identify::Behaviour::new(identify::Config::new(
                "/dbc/0.1.0".into(),
                key.public(),
            ));

            let upnp_behaviour = if enable_upnp {
                Toggle::from(Some(upnp::tokio::Behaviour::default()))
            } else {
                Toggle::from(None)
            };

            let mdns = if enable_mdns {
                Toggle::from(Some(mdns::tokio::Behaviour::new(
                    mdns::Config::default(),
                    key.public().to_peer_id(),
                )?))
            } else {
                Toggle::from(None)
            };

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(NodeBehaviour {
                gossipsub: gs,
                kad: kad_behaviour,
                identify,
                upnp: upnp_behaviour,
                mdns,
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    swarm.listen_on(cfg.listen.clone())?;

    if dial_peers {
        info!(
            "encrypted peer pool: {} dial target(s) — community nodes first, DNS fallback last",
            bootstrap.len()
        );
        dial_peer_list(&mut swarm, &bootstrap, "startup");
    } else {
        info!("listen-only — waiting for incoming peer connections");
    }

    if enable_dht {
        if let Err(e) = swarm.behaviour_mut().kad.bootstrap() {
            warn!("kad bootstrap: {e}");
        } else {
            info!("Kademlia DHT bootstrap started (protocol {KAD_PROTOCOL})");
        }
    }

    info!("P2P listening on {}", cfg.listen);
    info!("local peer id: {local_peer_id}");
    if let Ok(Some(tip)) = chain.tip() {
        info!("chain tip height={}", tip.height);
    }
    if bootstrap.is_empty() && enable_dht && dial_peers {
        info!("no peers in list — waiting for DHT / gossip");
    }
    if !enable_mdns {
        info!("mDNS disabled (use --mdns only on trusted LANs)");
    }
    if enable_upnp {
        info!("UPnP enabled — will try to open the listen port on your router automatically");
    } else {
        info!("UPnP disabled — inbound connections require manual port forwarding");
    }

    let mining_controlled = cfg.mine || cfg.mine_ctl_file.is_some();
    let (mine_tx, mut mine_rx) = tokio::sync::mpsc::channel::<()>(4);
    /// Wake mining loop immediately after syncing a block from the network.
    let mine_tx_wakeup = mine_tx.clone();

    if mining_controlled {
        // Create mine_ctl only if missing — the UI writes "1"/"0" before spawning; do not overwrite.
        if let Some(ref p) = cfg.mine_ctl_file {
            if !p.exists() {
                let _ = fs::write(p, if cfg.mine { "1" } else { "0" });
            }
        }

        let always_mine = cfg.mine;
        let mine_ctl_file = cfg.mine_ctl_file.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(
                crate::consensus::TARGET_BLOCK_TIME_SECS.min(30),
            ));

            loop {
                interval.tick().await;

                let enabled = if always_mine {
                    true
                } else if let Some(ref p) = mine_ctl_file {
                    match fs::read_to_string(p) {
                        Ok(s) => s.trim() == "1",
                        Err(_) => false,
                    }
                } else {
                    false
                };

                if enabled {
                    if mine_tx.send(()).await.is_err() {
                        break;
                    }
                }
            }
        });
    }

    let mut kad_refresh = tokio::time::interval(Duration::from_secs(300));
    let mut peer_search = tokio::time::interval(Duration::from_secs(30));
    let mut status_tick = tokio::time::interval(Duration::from_secs(5));
    let listen_port = cfg
        .listen
        .iter()
        .find_map(|p| {
            if let libp2p::multiaddr::Protocol::Tcp(port) = p {
                Some(port)
            } else {
                None
            }
        })
        .unwrap_or(8333);
    let status_data_dir = data_dir.to_path_buf();
    let status_mine_ctl = cfg.mine_ctl_file.clone();
    let status_always_mine = cfg.mine;
    let mut mining_task: Option<tokio::task::JoinHandle<Result<Block, MinerError>>> = None;
    let mut initial_peer_search_done = !dial_peers;

    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(NodeEvent::Gossipsub(gossipsub::Event::Message {
                        message,
                        ..
                    })) => {
                        if let Ok(msg) = decode(&message.data) {
                            let accepted = handle_network_message(
                                &mut swarm,
                                &chain,
                                &mut registry,
                                &peers_enc_path,
                                &blocks_topic,
                                &sync_topic,
                                &peers_topic,
                                msg,
                            )
                            .await?;
                            if accepted {
                                abort_mining_task(&mut mining_task);
                                let _ = mine_tx_wakeup.try_send(());
                            }
                        }
                    }
                    SwarmEvent::Behaviour(NodeEvent::Kad(kad::Event::RoutingUpdated { peer, .. })) => {
                        info!("kad routing updated: {peer}");
                    }
                    SwarmEvent::Behaviour(NodeEvent::Upnp(ev)) => match ev {
                        upnp::Event::NewExternalAddr(addr) => {
                            info!("upnp: router mapped external address {addr} — registering in peer pool");
                            let mut dialable = addr.clone();
                            use libp2p::multiaddr::Protocol;
                            if !dialable.iter().any(|p| matches!(p, Protocol::Tcp(_))) {
                                dialable.push(Protocol::Tcp(listen_port));
                            }
                            if let Ok(with_peer) = dialable.with_p2p(local_peer_id) {
                                if registry.add_multiaddr(&with_peer) {
                                    let _ = registry.save_encrypted(&peers_enc_path);
                                    kad_add_pool_addresses(&mut swarm, &registry);
                                    let _ = publish_peer_list(&mut swarm, &registry, &peers_topic);
                                }
                            }
                        }
                        upnp::Event::ExpiredExternalAddr(addr) => {
                            warn!("upnp: external address {addr} expired");
                        }
                        upnp::Event::GatewayNotFound => {
                            warn!("upnp: no UPnP gateway found — outbound-only operation still works");
                        }
                        upnp::Event::NonRoutableGateway => {
                            warn!("upnp: router has no public IP (CGNAT?) — outbound-only operation still works");
                        }
                    },
                    SwarmEvent::Behaviour(NodeEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                        for addr in info.listen_addrs {
                            swarm.behaviour_mut().kad.add_address(&peer_id, addr.clone());
                            let with_peer = addr.clone().with_p2p(peer_id).unwrap_or_else(|_| addr.clone());
                            if registry.record_reachable_peer(&with_peer) {
                                let _ = registry.save_encrypted(&peers_enc_path);
                            }
                            if !swarm.connected_peers().any(|p| *p == peer_id) {
                                if let Err(e) = swarm.dial(with_peer) {
                                    warn!("dial peer {peer_id} via identify failed: {e}");
                                }
                            }
                        }
                        kad_add_pool_addresses(&mut swarm, &registry);
                    }
                    SwarmEvent::Behaviour(NodeEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer, addr) in list {
                            info!("mdns discovered {peer} at {addr}");
                            let _ = swarm.dial(addr);
                        }
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!("listening on {address}");
                        info!("share this reachability-safe address only if you choose to seed (use VPN/VPS, not home IP for anonymity)");
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        info!("connected to {peer_id}");
                        let remote = match endpoint {
                            libp2p::core::connection::ConnectedPoint::Dialer { address, .. } => {
                                address.clone()
                            }
                            libp2p::core::connection::ConnectedPoint::Listener {
                                send_back_addr,
                                ..
                            } => send_back_addr.clone(),
                        };
                        let with_peer = remote
                            .clone()
                            .with_p2p(peer_id)
                            .unwrap_or(remote);
                        if registry.record_reachable_peer(&with_peer) {
                            let _ = registry.save_encrypted(&peers_enc_path);
                            info!(
                                "encrypted peer pool — {} reachable node(s) stored locally",
                                registry.len()
                            );
                            kad_add_pool_addresses(&mut swarm, &registry);
                        }
                        let _ = publish_peer_list(&mut swarm, &registry, &peers_topic);
                        let _ = request_next_block(&mut swarm, &chain, &sync_topic);
                    }
                    _ => {}
                }
            }
            _ = kad_refresh.tick(), if enable_dht => {
                let _ = swarm.behaviour_mut().kad.bootstrap();
            }
            _ = peer_search.tick(), if dial_peers => {
                initial_peer_search_done = true;
                bootstrap = build_dial_targets(
                    &registry,
                    &extra_bootstrap,
                    peers_file.as_deref(),
                );
                let connected = swarm.connected_peers().next().is_some();
                if connected {
                    let _ = request_next_block(&mut swarm, &chain, &sync_topic);
                    let _ = publish_peer_list(&mut swarm, &registry, &peers_topic);
                } else {
                    info!("searching for peers — {} dial target(s)", bootstrap.len());
                    dial_peer_list(&mut swarm, &bootstrap, "periodic");
                }
            }
            _ = status_tick.tick() => {
                let peer_count = swarm.connected_peers().count() as u32;
                let tip_height = chain.tip().ok().flatten().map(|t| t.height);
                let mining_enabled = mining_ctl_enabled(status_always_mine, &status_mine_ctl);
                let policy = mining_policy(&swarm, mining_enabled);
                // While peers are connected, re-ask for the next block every few seconds so we
                // catch up if another miner found it while we were grinding (gossip can be missed).
                if peer_count > 0 {
                    if let Ok(next) = chain.tip().map(|t| t.map(|x| x.height + 1).unwrap_or(0)) {
                        info!("sync — asking network for block height={next}");
                    }
                    let _ = request_next_block(&mut swarm, &chain, &sync_topic);
                }
                let _ = write_status(
                    &status_data_dir,
                    &NodeStatusSnapshot {
                        tip_height,
                        peer_count,
                        peer_pool_size: registry.len() as u32,
                        mining_enabled,
                        listening: true,
                        mining_mode: policy.status_mode().to_string(),
                    },
                );
            }
            Some(()) = mine_rx.recv(), if mining_task.is_none() && cfg.payout.is_some() => {
                let mining_enabled = mining_ctl_enabled(status_always_mine, &status_mine_ctl);
                let policy = mining_policy(&swarm, mining_enabled);
                if policy.should_mine() && initial_peer_search_done {
                    if let Some(payout) = cfg.payout {
                        let peer_n = swarm.connected_peers().count();
                        // Sync before grinding so a long-running solo session picks up blocks
                        // found by others since the last connect event.
                        let _ = request_next_block(&mut swarm, &chain, &sync_topic);
                        match prepare_mine_job(&chain, payout) {
                            Ok(job) => {
                                info!("{}", policy.mining_log_line(job.height, peer_n));
                                mining_task = Some(tokio::task::spawn_blocking(move || {
                                    Miner::mine_next_block(
                                        job.prev_hash,
                                        job.height,
                                        job.difficulty,
                                        job.payout,
                                        GENESIS_MSG,
                                        job.candidate_txs,
                                        job.uncles,
                                    )
                                }));
                            }
                            Err(e) => warn!("mine prep failed: {e}"),
                        }
                    }
                }
            }
            mine_result = async {
                if let Some(h) = mining_task.as_mut() {
                    h.await
                } else {
                    std::future::pending().await
                }
            }, if mining_task.is_some() => {
                mining_task = None;
                match mine_result {
                    Ok(Ok(block)) => {
                        if let Err(e) =
                            finish_mined_block(&mut swarm, &chain, block, &blocks_topic).await
                        {
                            warn!("mine publish failed: {e}");
                        }
                    }
                    Ok(Err(e)) => warn!("mine failed: {e}"),
                    Err(e) => warn!("mine task panicked: {e}"),
                }
            }
        }
    }
}

struct MineJob {
    prev_hash: Hash,
    height: u64,
    difficulty: u32,
    payout: Address,
    candidate_txs: Vec<(crate::Transaction, u64)>,
    uncles: Vec<crate::BlockHeader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MiningPolicy {
    Off,
    Solo,
    Network,
}

impl MiningPolicy {
    fn should_mine(self) -> bool {
        !matches!(self, MiningPolicy::Off)
    }

    fn status_mode(self) -> &'static str {
        match self {
            MiningPolicy::Off => "off",
            MiningPolicy::Solo => "solo",
            MiningPolicy::Network => "network",
        }
    }

    fn mining_log_line(self, height: u64, peer_count: usize) -> String {
        match self {
            MiningPolicy::Solo => format!(
                "solo mining block height={height} (BritishWork — can take minutes)…"
            ),
            MiningPolicy::Network => format!(
                "network mining block height={height} with {peer_count} peer(s) — all miners grind, first block wins…"
            ),
            MiningPolicy::Off => String::new(),
        }
    }
}

fn mining_ctl_enabled(always_mine: bool, mine_ctl_file: &Option<PathBuf>) -> bool {
    if always_mine {
        return true;
    }
    if let Some(p) = mine_ctl_file {
        return fs::read_to_string(p)
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
    }
    false
}

fn mining_policy(swarm: &libp2p::Swarm<NodeBehaviour>, ctl_enabled: bool) -> MiningPolicy {
    if !ctl_enabled {
        return MiningPolicy::Off;
    }
    if swarm.connected_peers().next().is_none() {
        MiningPolicy::Solo
    } else {
        MiningPolicy::Network
    }
}

fn abort_mining_task(task: &mut Option<tokio::task::JoinHandle<Result<Block, MinerError>>>) {
    if let Some(handle) = task.take() {
        handle.abort();
    }
}

fn prepare_mine_job(chain: &Chain, payout: Address) -> anyhow::Result<MineJob> {
    Ok(MineJob {
        prev_hash: chain.tip()?.map(|t| t.hash).unwrap_or(Hash::ZERO),
        height: chain.tip()?.map(|t| t.height + 1).unwrap_or(0),
        difficulty: chain.difficulty_for_next_block()?,
        candidate_txs: chain.mempool_snapshot_with_fees(),
        uncles: chain.select_uncles()?,
        payout,
    })
}

async fn finish_mined_block(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    chain: &Chain,
    block: Block,
    blocks_topic: &IdentTopic,
) -> anyhow::Result<()> {
    let height = block.header.height;
    if let Some(hash) = chain.accept_block(&block)? {
        info!("mined block height={height} hash={}", hash.to_hex());
        publish(swarm, blocks_topic, &NetworkMessage::Block(block))?;
        request_next_block(swarm, chain, &IdentTopic::new(TOPIC_SYNC))?;
    }
    Ok(())
}

fn peer_id_from_multiaddr(addr: &Multiaddr) -> Option<PeerId> {
    use libp2p::multiaddr::Protocol;
    addr.iter().find_map(|p| {
        if let Protocol::P2p(peer_id) = p {
            Some(peer_id)
        } else {
            None
        }
    })
}

async fn handle_network_message(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    chain: &Chain,
    registry: &mut PeerRegistry,
    peers_enc_path: &Path,
    blocks_topic: &IdentTopic,
    sync_topic: &IdentTopic,
    peers_topic: &IdentTopic,
    msg: NetworkMessage,
) -> anyhow::Result<bool> {
    let mut accepted_new = false;
    match msg {
        NetworkMessage::Block(block) => {
            match chain.accept_block(&block) {
                Ok(Some(hash)) => {
                    info!(
                        "accepted block height={} hash={}",
                        block.header.height,
                        hash.to_hex()
                    );
                    request_next_block(swarm, chain, sync_topic)?;
                    accepted_new = true;
                }
                Ok(None) => {}
                Err(e) => warn!("rejected block: {e}"),
            }
        }
        NetworkMessage::Tx(tx) => match chain.add_mempool_tx(tx) {
            Ok(()) => info!("mempool tx accepted"),
            Err(e) => warn!("mempool reject: {e}"),
        },
        NetworkMessage::GetBlock { height } => {
            if let Ok(Some(block)) = chain.db().get_block_at_height(height) {
                let reply = NetworkMessage::BlockReply {
                    height,
                    block: Some(block),
                };
                publish(swarm, sync_topic, &reply)?;
            }
        }
        NetworkMessage::BlockReply { height, block } => {
            if let Some(block) = block {
                if chain.accept_block(&block)?.is_some() {
                    info!("synced block height={height}");
                    request_next_block(swarm, chain, sync_topic)?;
                    accepted_new = true;
                }
            }
        }
        NetworkMessage::PeerList { peers } => {
            let added = registry.merge_peer_entries(&peers);
            if added > 0 {
                registry.save_encrypted(peers_enc_path)?;
                info!(
                    "encrypted peer pool merged — {} reachable node(s) in bootstrap pool",
                    registry.len()
                );
                kad_add_pool_addresses(swarm, registry);
                for entry in peers {
                    if let Ok(addr) = entry.multiaddr.parse::<Multiaddr>() {
                        if let Err(e) = swarm.dial(addr) {
                            warn!("dial {} failed: {e}", entry.multiaddr);
                        }
                    }
                }
            }
            publish_peer_list(swarm, registry, peers_topic)?;
        }
    }
    let _ = blocks_topic;
    Ok(accepted_new)
}

fn dial_peer_list(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    addrs: &[Multiaddr],
    reason: &str,
) {
    info!(
        "searching encrypted peer pool ({reason}) — {} dial target(s)",
        addrs.len()
    );
    for addr in addrs {
        if let Err(e) = swarm.dial(addr.clone()) {
            warn!("dial {addr} failed: {e}");
        }
    }
}

fn publish_peer_list(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    registry: &PeerRegistry,
    peers_topic: &IdentTopic,
) -> anyhow::Result<()> {
    let msg = NetworkMessage::PeerList {
        peers: registry.entries().to_vec(),
    };
    publish(swarm, peers_topic, &msg)
}

fn build_dial_targets(
    registry: &PeerRegistry,
    extra_bootstrap: &[Multiaddr],
    peers_file: Option<&Path>,
) -> Vec<Multiaddr> {
    let mut out = registry.dial_order();
    for addr in extra_bootstrap {
        let s = addr.to_string();
        if out.iter().any(|b| b.to_string() == s) {
            continue;
        }
        out.push(addr.clone());
    }
    if let Some(path) = peers_file {
        if let Ok(from_file) = load_peers_file(path) {
            for addr in from_file {
                let s = addr.to_string();
                if !out.iter().any(|b| b.to_string() == s) {
                    out.push(addr);
                }
            }
        }
    }
    out
}

fn kad_add_pool_addresses(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    registry: &PeerRegistry,
) {
    for addr in registry.dial_order() {
        if let Some(peer) = peer_id_from_multiaddr(&addr) {
            let dial_addr = addr.clone().with_p2p(peer).unwrap_or_else(|_| addr.clone());
            swarm.behaviour_mut().kad.add_address(&peer, dial_addr);
        }
    }
}

fn request_next_block(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    chain: &Chain,
    sync_topic: &IdentTopic,
) -> anyhow::Result<()> {
    let next = chain.tip()?.map(|t| t.height + 1).unwrap_or(0);
    let msg = NetworkMessage::GetBlock { height: next };
    publish(swarm, sync_topic, &msg)
}

fn publish(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    topic: &IdentTopic,
    msg: &NetworkMessage,
) -> anyhow::Result<()> {
    let data = encode(msg)?;
    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.clone(), data)?;
    Ok(())
}

fn load_or_create_key(path: &Path) -> anyhow::Result<(Keypair, bool)> {
    if path.exists() {
        let bytes = std::fs::read(path)?;
        return Ok((Keypair::from_protobuf_encoding(&bytes)?, false));
    }
    let kp = Keypair::generate_ed25519();
    std::fs::write(path, kp.to_protobuf_encoding()?)?;
    Ok((kp, true))
}

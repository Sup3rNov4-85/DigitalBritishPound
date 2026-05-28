use std::path::Path;
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
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, SwarmBuilder,
};
use tracing::{info, warn};

use crate::{
    crypto::wallet::Address,
    network::{
        protocol::{decode, encode, NetworkMessage, TOPIC_BLOCKS, TOPIC_SYNC, TOPIC_TXS},
        seeds::load_peers_file,
    },
    node::{chain::Chain, miner::Miner},
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
    /// LAN-only; off by default so your node does not advertise on local multicast.
    mdns: Toggle<mdns::tokio::Behaviour>,
}

#[derive(Debug)]
enum NodeEvent {
    Gossipsub(gossipsub::Event),
    Kad(kad::Event),
    Identify(identify::Event),
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

impl From<mdns::Event> for NodeEvent {
    fn from(e: mdns::Event) -> Self {
        NodeEvent::Mdns(e)
    }
}

pub struct P2pConfig {
    pub listen: Multiaddr,
    /// Explicit peers (CLI + peers file). Should be community-operated, not founder home IP.
    pub bootstrap: Vec<Multiaddr>,
    pub peers_file: Option<std::path::PathBuf>,
    pub mine: bool,
    pub payout: Option<Address>,
    /// mDNS is for local dev only; keep false for anonymous public mining.
    pub enable_mdns: bool,
    pub enable_dht: bool,
}

pub async fn run_p2p(data_dir: &Path, chain: Chain, cfg: P2pConfig) -> anyhow::Result<()> {
    let key_path = data_dir.join("peer_key");
    let key = load_or_create_key(&key_path)?;
    let local_peer_id = key.public().to_peer_id();

    let blocks_topic = IdentTopic::new(TOPIC_BLOCKS);
    let txs_topic = IdentTopic::new(TOPIC_TXS);
    let sync_topic = IdentTopic::new(TOPIC_SYNC);

    let mut bootstrap = cfg.bootstrap.clone();
    if let Some(ref path) = cfg.peers_file {
        bootstrap.extend(load_peers_file(path)?);
    }

    let enable_mdns = cfg.enable_mdns;
    let enable_dht = cfg.enable_dht;

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
                mdns,
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    swarm.listen_on(cfg.listen.clone())?;

    for addr in &bootstrap {
        if let Err(e) = swarm.dial(addr.clone()) {
            warn!("dial {addr} failed: {e}");
        }
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
    if bootstrap.is_empty() && enable_dht {
        info!("no explicit peers — waiting for DHT / gossip from other independent nodes");
    }
    if !enable_mdns {
        info!("mDNS disabled (use --mdns only on trusted LANs)");
    }

    let (mine_tx, mut mine_rx) = tokio::sync::mpsc::channel::<()>(4);
    if cfg.mine {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(
                crate::consensus::TARGET_BLOCK_TIME_SECS.min(30),
            ));
            loop {
                interval.tick().await;
                if mine_tx.send(()).await.is_err() {
                    break;
                }
            }
        });
    }

    let mut kad_refresh = tokio::time::interval(Duration::from_secs(300));

    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(NodeEvent::Gossipsub(gossipsub::Event::Message {
                        message,
                        ..
                    })) => {
                        if let Ok(msg) = decode(&message.data) {
                            handle_network_message(&mut swarm, &chain, &blocks_topic, &sync_topic, msg).await?;
                        }
                    }
                    SwarmEvent::Behaviour(NodeEvent::Kad(kad::Event::RoutingUpdated { peer, .. })) => {
                        info!("kad routing updated: {peer}");
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
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        info!("connected to {peer_id}");
                        request_next_block(&mut swarm, &chain, &sync_topic)?;
                    }
                    _ => {}
                }
            }
            _ = kad_refresh.tick(), if enable_dht => {
                let _ = swarm.behaviour_mut().kad.bootstrap();
            }
            Some(()) = mine_rx.recv(), if cfg.mine => {
                if let Some(payout) = cfg.payout {
                    if let Err(e) = try_mine_and_publish(&mut swarm, &chain, payout, &blocks_topic).await {
                        warn!("mine failed: {e}");
                    }
                }
            }
        }
    }
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
    blocks_topic: &IdentTopic,
    sync_topic: &IdentTopic,
    msg: NetworkMessage,
) -> anyhow::Result<()> {
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
            let block = chain.db().get_block_at_height(height)?;
            let reply = NetworkMessage::BlockReply { height, block };
            publish(swarm, sync_topic, &reply)?;
        }
        NetworkMessage::BlockReply { height, block } => {
            if let Some(block) = block {
                if chain.accept_block(&block)?.is_some() {
                    info!("synced block height={height}");
                    request_next_block(swarm, chain, sync_topic)?;
                }
            }
        }
    }
    let _ = blocks_topic;
    Ok(())
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

async fn try_mine_and_publish(
    swarm: &mut libp2p::Swarm<NodeBehaviour>,
    chain: &Chain,
    payout: Address,
    blocks_topic: &IdentTopic,
) -> anyhow::Result<()> {
    let prev_hash = chain.tip()?.map(|t| t.hash).unwrap_or(crate::Hash::ZERO);
    let height = chain.tip()?.map(|t| t.height + 1).unwrap_or(0);
    let difficulty = chain.difficulty_for_next_block()?;
    let txs = chain.mempool_snapshot();
    let fees = chain.mempool_fees()?;
    let uncles = chain.select_uncles()?;

    let block = Miner::mine_next_block(
        prev_hash,
        height,
        difficulty,
        payout,
        GENESIS_MSG,
        txs,
        fees,
        uncles,
    )?;

    if let Some(hash) = chain.accept_block(&block)? {
        info!("mined block height={height} hash={}", hash.to_hex());
        publish(swarm, blocks_topic, &NetworkMessage::Block(block))?;
        request_next_block(swarm, chain, &IdentTopic::new(TOPIC_SYNC))?;
    }
    Ok(())
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

fn load_or_create_key(path: &Path) -> anyhow::Result<Keypair> {
    if path.exists() {
        let bytes = std::fs::read(path)?;
        return Ok(Keypair::from_protobuf_encoding(&bytes)?);
    }
    let kp = Keypair::generate_ed25519();
    std::fs::write(path, kp.to_protobuf_encoding()?)?;
    Ok(kp)
}

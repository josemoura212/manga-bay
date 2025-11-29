use crate::storage::engine::Storage;
use anyhow::Result;
use libp2p::futures::StreamExt;
use libp2p::identity;
use libp2p::{
    gossipsub, identify, kad, mdns, noise, request_response, tcp, yamux, Multiaddr, PeerId,
    SwarmBuilder,
};
use std::time::Duration;

use tokio::select;

use tokio::sync::{mpsc, oneshot};

#[derive(Debug)]
pub struct NodeInfo {
    pub peer_id: String,
    pub listeners: Vec<String>,
    pub connected_peers: Vec<String>,
}

#[derive(Debug)]
pub enum NodeCommand {
    GetNodeInfo(oneshot::Sender<NodeInfo>),
    ConnectPeer(Multiaddr, oneshot::Sender<Result<()>>),
    FindManga(
        String,
        oneshot::Sender<Option<crate::storage::models::MangaMetadata>>,
    ),
}

pub async fn run(port: u16, storage: Storage, bootstrap_peers: Vec<String>) -> Result<()> {
    // 1. Load or Generate Identity
    let id_keys = load_or_generate_keypair(&storage.data_dir).await?;
    let local_peer_id = PeerId::from(id_keys.public());
    tracing::info!("Local PeerId: {local_peer_id}");

    // 4. Build Swarm
    let mut swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_behaviour(|key| {
            let peer_id = PeerId::from(key.public());
            Ok(crate::p2p::behaviour::MangaBehaviour {
                gossipsub: gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub::Config::default(),
                )
                .map_err(|e| anyhow::anyhow!(e))?,
                kademlia: kad::Behaviour::new(peer_id, kad::store::MemoryStore::new(peer_id)),
                identify: identify::Behaviour::new(identify::Config::new(
                    "/manga-bay/1.0.0".into(),
                    key.public(),
                )),
                mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)?,
                request_response: request_response::cbor::Behaviour::new(
                    [(
                        libp2p::StreamProtocol::new("/manga-bay/req/1.0.0"),
                        request_response::ProtocolSupport::Full,
                    )],
                    request_response::Config::default(),
                ),
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    // 5. Listen
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", port + 1).parse()?)?;

    // 6. Bootstrap
    let persistence = crate::node::discovery::PeerPersistence::new(&storage.data_dir);
    let mut known_peers = persistence.load().await.unwrap_or_default();

    // Add CLI bootstrap peers
    for peer_addr in bootstrap_peers {
        if let Ok(addr) = peer_addr.parse::<Multiaddr>() {
            known_peers.push(addr);
        }
    }

    if !known_peers.is_empty() {
        tracing::info!("Bootstrapping with {} peers...", known_peers.len());
        for addr in &known_peers {
            if let Err(e) = swarm.dial(addr.clone()) {
                tracing::warn!("Failed to dial peer: {}", e);
            }
        }
    }

    // 7. Ratio Manager
    let ratio_manager = crate::node::ratio::RatioManager::new(storage.pool.clone());
    ratio_manager.init().await?;

    // 8. Resource Manager
    let resource_manager = crate::node::resources::ResourceManager::new(storage.data_dir.clone());
    resource_manager.start_monitoring().await;

    // Command Channel
    let (cmd_tx, mut cmd_rx) = mpsc::channel(32);

    // Pending Requests Map: MangaID -> List of waiters
    let mut pending_manga_requests: std::collections::HashMap<
        String,
        Vec<oneshot::Sender<Option<crate::storage::models::MangaMetadata>>>,
    > = std::collections::HashMap::new();

    // 9. Run Loop
    let api_server = crate::api::server::serve(port, storage.clone(), cmd_tx);
    let mut save_interval = tokio::time::interval(Duration::from_secs(60)); // Save peers every minute

    select! {
        _ = api_server => {},
        _ = async {
            loop {
                select! {
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            NodeCommand::GetNodeInfo(reply) => {
                                let listeners: Vec<String> = swarm.listeners().map(|a| a.to_string()).collect();
                                let connected_peers: Vec<String> = swarm.connected_peers().map(|p| p.to_string()).collect();
                                let info = NodeInfo {
                                    peer_id: local_peer_id.to_string(),
                                    listeners,
                                    connected_peers,
                                };
                                let _ = reply.send(info);
                            }
                            NodeCommand::ConnectPeer(addr, reply) => {
                                let result = swarm.dial(addr).map_err(|e| anyhow::anyhow!(e));
                                let _ = reply.send(result);
                            }
                            NodeCommand::FindManga(manga_id, reply) => {
                                // Add to pending
                                pending_manga_requests.entry(manga_id.clone()).or_default().push(reply);

                                // Broadcast request to all connected peers (Flood)
                                // In a real DHT, we would query the DHT.
                                let peers: Vec<_> = swarm.connected_peers().cloned().collect();
                                if peers.is_empty() {
                                    tracing::warn!("No peers connected to search for manga {}", manga_id);
                                    // Fail immediately if no peers
                                    if let Some(waiters) = pending_manga_requests.remove(&manga_id) {
                                        for tx in waiters {
                                            let _ = tx.send(None);
                                        }
                                    }
                                } else {
                                    tracing::info!("Querying {} peers for manga {}", peers.len(), manga_id);
                                    let req = crate::p2p::protocol::AppRequest::GetManga { manga_id: manga_id.clone() };
                                    for peer in peers {
                                        swarm.behaviour_mut().request_response.send_request(&peer, req.clone());
                                    }

                                    // Timeout fallback (simple implementation)
                                    // Ideally we track request IDs, but for MVP we just rely on response or manual timeout in API
                                }
                            }
                        }
                    }
                    _ = save_interval.tick() => {
                        // In a real implementation, we would extract connected peers from the swarm
                        // For now, we just save the initial list + any we might add
                        // TODO: Extract peers from Kademlia buckets
                        if let Err(e) = persistence.save(known_peers.clone()).await {
                            tracing::error!("Failed to save peers: {}", e);
                        }
                    }
                    event = swarm.select_next_some() => {
                        match event {
                            libp2p::swarm::SwarmEvent::Behaviour(crate::p2p::behaviour::MangaBehaviourEvent::RequestResponse(
                                request_response::Event::Message { message, peer, .. }
                            )) => {
                                match message {
                                    request_response::Message::Request { request, channel, .. } => {
                                        match request {
                                            crate::p2p::protocol::AppRequest::GetBlock { block_hash } => {
                                                tracing::info!("Received request for block: {}", block_hash);
                                                let data = match storage.get_chunk(&block_hash).await {
                                                    Ok(d) => d,
                                                    Err(e) => {
                                                        tracing::error!("Storage error: {:?}", e);
                                                        None
                                                    }
                                                };
                                                let size = data.as_ref().map(|d| d.len()).unwrap_or(0) as u64;
                                                let response = crate::p2p::protocol::AppResponse::Block(data);

                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                } else if size > 0 {
                                                    if let Err(e) = ratio_manager.record_upload(&peer.to_string(), size).await {
                                                        tracing::error!("Failed to record upload: {:?}", e);
                                                    }
                                                }
                                            }
                                            crate::p2p::protocol::AppRequest::GetManga { manga_id } => {
                                                tracing::info!("Received request for manga: {}", manga_id);
                                                let manga = match storage.get_manga(&manga_id).await {
                                                    Ok(m) => m,
                                                    Err(e) => {
                                                        tracing::error!("Storage error: {:?}", e);
                                                        None
                                                    }
                                                };
                                                let response = crate::p2p::protocol::AppResponse::Manga(manga);
                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                }
                                            }
                                        }
                                    }
                                    request_response::Message::Response { response, .. } => {
                                        match response {
                                            crate::p2p::protocol::AppResponse::Block(data) => {
                                                let size = data.as_ref().map(|d| d.len()).unwrap_or(0) as u64;
                                                tracing::info!("Received block response: {} bytes", size);
                                                if size > 0 {
                                                    if let Err(e) = ratio_manager.record_download(&peer.to_string(), size).await {
                                                        tracing::error!("Failed to record download: {:?}", e);
                                                    }
                                                }
                                            }
                                            crate::p2p::protocol::AppResponse::Manga(Some(manga)) => {
                                                tracing::info!("Received manga metadata: {}", manga.title);
                                                // Resolve pending requests
                                                if let Some(waiters) = pending_manga_requests.remove(&manga.id) {
                                                    // We only need to send to one, but let's send to all for now (or just the first and drop others)
                                                    // Actually, we should clone the result to all
                                                    for tx in waiters {
                                                        let _ = tx.send(Some(manga.clone()));
                                                    }
                                                }

                                                // Save to local storage?
                                                // For now, we return it to API, API can decide to save.
                                            }
                                            crate::p2p::protocol::AppResponse::Manga(None) => {
                                                tracing::info!("Peer did not have the manga");
                                            }
                                        }
                                    }
                                }
                            }
                            event => tracing::info!("Swarm Event: {:?}", event),
                        }
                    }
                }
            }
        } => {}
    }

    Ok(())
}

async fn load_or_generate_keypair(data_dir: &std::path::Path) -> Result<identity::Keypair> {
    let key_path = data_dir.join("identity.key");

    if key_path.exists() {
        tracing::info!("Loading identity from {:?}", key_path);
        let mut bytes = tokio::fs::read(&key_path).await?;
        match identity::Keypair::from_protobuf_encoding(&bytes) {
            Ok(keypair) => return Ok(keypair),
            Err(e) => {
                tracing::warn!("Failed to decode identity key: {}. Generating new one.", e);
            }
        }
    }

    tracing::info!("Generating new identity and saving to {:?}", key_path);
    let keypair = identity::Keypair::generate_ed25519();
    let bytes = keypair.to_protobuf_encoding()?;
    tokio::fs::write(&key_path, bytes).await?;

    Ok(keypair)
}

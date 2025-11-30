use anyhow::Result;
use libp2p::futures::StreamExt;
use libp2p::identity;
use libp2p::multiaddr::Protocol;
use libp2p::{
    dcutr, gossipsub, identify, kad, mdns, noise, relay, request_response, tcp, yamux, Multiaddr,
    PeerId, SwarmBuilder,
};
use manga_bay_storage::Storage;
use std::collections::HashSet;
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
        oneshot::Sender<Option<manga_bay_common::models::MangaMetadata>>,
    ),
    FindChapters(
        String,
        oneshot::Sender<Vec<manga_bay_common::models::ChapterMetadata>>,
    ),
    FindChapterDetails(
        String,
        oneshot::Sender<Option<manga_bay_common::models::ChapterDetails>>,
    ),
    FindBlock(String, oneshot::Sender<Option<Vec<u8>>>),
    SyncManga(
        String,
        oneshot::Sender<Vec<manga_bay_common::models::MangaVersion>>,
    ),
    DiscoverPeers(oneshot::Sender<usize>),
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
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|key, relay_client| {
            let peer_id = PeerId::from(key.public());
            Ok(manga_bay_p2p::behaviour::MangaBehaviour {
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
                mdns: libp2p::swarm::behaviour::toggle::Toggle::from(
                    match mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id) {
                        Ok(m) => Some(m),
                        Err(e) => {
                            tracing::warn!(
                                "mDNS failed to start (likely due to server environment): {}",
                                e
                            );
                            None
                        }
                    },
                ),
                request_response: request_response::cbor::Behaviour::new(
                    [(
                        libp2p::StreamProtocol::new("/manga-bay/req/1.0.0"),
                        request_response::ProtocolSupport::Full,
                    )],
                    request_response::Config::default(),
                ),
                relay: relay_client,
                relay_server: relay::Behaviour::new(peer_id, relay::Config::default()),
                dcutr: dcutr::Behaviour::new(peer_id),
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();
    tracing::info!("Swarm built successfully");

    // 5. Listen
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", port + 1).parse()?)?;
    tracing::info!("Swarm listening on port {}", port + 1);

    // 6. Bootstrap
    let persistence = crate::discovery::PeerPersistence::new(&storage.data_dir);
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
    let ratio_manager = crate::ratio::RatioManager::new(storage.pool.clone());
    ratio_manager.init().await?;

    // 8. Resource Manager
    let resource_manager = crate::resources::ResourceManager::new(storage.data_dir.clone());
    resource_manager.start_monitoring().await;

    // Command Channel
    let (cmd_tx, mut cmd_rx) = mpsc::channel(32);

    // 9. Run Loop
    let mut node_state = crate::events::NodeState::new();
    node_state.known_peers = known_peers;

    let api_server = crate::api::server::serve(port, storage.clone(), cmd_tx);
    let mut save_interval = tokio::time::interval(Duration::from_secs(60)); // Save peers every minute

    tracing::info!("Starting run loop...");
    select! {
        res = api_server => {
            tracing::error!("API server exited: {:?}", res);
        },
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
                                let result = swarm.dial(addr).map_err(|e: libp2p::swarm::DialError| anyhow::anyhow!(e));
                                let _ = reply.send(result);
                            }
                            NodeCommand::FindManga(manga_id, reply) => {
                                // Add to pending
                                node_state.pending_manga_requests.entry(manga_id.clone()).or_default().push(reply);

                                // 1. Send to currently connected peers
                                let connected_peers: Vec<_> = swarm.connected_peers().cloned().collect();
                                if !connected_peers.is_empty() {
                                    tracing::info!("Querying {} connected peers for manga {}", connected_peers.len(), manga_id);
                                    let req = manga_bay_p2p::protocol::AppRequest::GetManga { manga_id: manga_id.clone() };
                                    for peer in &connected_peers {
                                        swarm.behaviour_mut().request_response.send_request(peer, req.clone());
                                    }
                                }

                                // 2. Dial known but disconnected peers
                                let mut dialed_count = 0;
                                for addr in &node_state.known_peers {
                                    // We can't easily check if an addr belongs to a connected peer without parsing,
                                    // but calling dial on an already connected peer is usually fine (no-op or ignored).
                                    // However, to be cleaner, we could try to track peer_id -> addr.
                                    // For now, just dial.
                                    if swarm.dial(addr.clone()).is_err() {
                                        // Ignore errors (e.g. invalid addr or already dialing)
                                    } else {
                                        dialed_count += 1;
                                    }
                                }

                                if connected_peers.is_empty() && dialed_count == 0 {
                                     tracing::warn!("No peers to query for manga {}", manga_id);
                                     // Fail immediately if no one to talk to
                                     if let Some(waiters) = node_state.pending_manga_requests.remove(&manga_id) {
                                         for tx in waiters {
                                             let _ = tx.send(None);
                                         }
                                     }
                                } else if dialed_count > 0 {
                                    tracing::info!("Dialing {} known peers to search for manga {}", dialed_count, manga_id);
                                }
                            }
                            NodeCommand::FindChapters(manga_id, reply) => {
                                node_state.pending_chapters_requests.entry(manga_id.clone()).or_default().push(reply);
                                let connected_peers: Vec<_> = swarm.connected_peers().cloned().collect();
                                if !connected_peers.is_empty() {
                                    tracing::info!("Querying {} connected peers for chapters of {}", connected_peers.len(), manga_id);
                                    let req = manga_bay_p2p::protocol::AppRequest::GetChapters { manga_id: manga_id.clone() };
                                    for peer in &connected_peers {
                                        swarm.behaviour_mut().request_response.send_request(peer, req.clone());
                                    }
                                }
                            }
                            NodeCommand::FindChapterDetails(chapter_id, reply) => {
                                node_state.pending_chapter_details_requests.entry(chapter_id.clone()).or_default().push(reply);
                                let connected_peers: Vec<_> = swarm.connected_peers().cloned().collect();
                                if !connected_peers.is_empty() {
                                    tracing::info!("Querying {} connected peers for chapter details {}", connected_peers.len(), chapter_id);
                                    let req = manga_bay_p2p::protocol::AppRequest::GetChapterDetails { chapter_id: chapter_id.clone() };
                                    for peer in &connected_peers {
                                        swarm.behaviour_mut().request_response.send_request(peer, req.clone());
                                    }
                                }
                            }
                            NodeCommand::FindBlock(hash, reply) => {
                                node_state.pending_block_requests.entry(hash.clone()).or_default().push(reply);
                                let connected_peers: Vec<_> = swarm.connected_peers().cloned().collect();
                                if !connected_peers.is_empty() {
                                    tracing::info!("Querying {} connected peers for block {}", connected_peers.len(), hash);
                                    let req = manga_bay_p2p::protocol::AppRequest::GetBlock { block_hash: hash.clone() };
                                    for peer in &connected_peers {
                                        swarm.behaviour_mut().request_response.send_request(peer, req.clone());
                                    }
                                }
                            }
                            NodeCommand::SyncManga(manga_id, reply) => {
                                node_state.pending_version_requests.entry(manga_id.clone()).or_default().push(reply);
                                let connected_peers: Vec<_> = swarm.connected_peers().cloned().collect();
                                if !connected_peers.is_empty() {
                                    tracing::info!("Querying {} connected peers for version of {}", connected_peers.len(), manga_id);
                                    let req = manga_bay_p2p::protocol::AppRequest::GetVersion { manga_id: manga_id.clone() };
                                    for peer in &connected_peers {
                                        swarm.behaviour_mut().request_response.send_request(peer, req.clone());
                                    }
                                }
                            }
                            NodeCommand::DiscoverPeers(reply) => {
                                let connected_peers: Vec<_> = swarm.connected_peers().cloned().collect();
                                let count = connected_peers.len();
                                if !connected_peers.is_empty() {
                                    tracing::info!("Asking {} peers for their known peers", count);
                                    let req = manga_bay_p2p::protocol::AppRequest::GetPeers;
                                    for peer in &connected_peers {
                                        swarm.behaviour_mut().request_response.send_request(peer, req.clone());
                                    }
                                }
                                let _ = reply.send(count);
                            }
                        }
                    }
                    _ = save_interval.tick() => {
                        if let Err(e) = persistence.save(node_state.known_peers.clone()).await {
                            tracing::error!("Failed to save peers: {}", e);
                        }
                    }
                    event = swarm.select_next_some() => {
                        match event {
                            libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                                tracing::info!("Listening on {address:?}");
                            }
                            libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                                tracing::info!("Connection established with {}", peer_id);
                                // Add to known peers if not present
                                let addr = match endpoint {
                                    libp2p::core::ConnectedPoint::Dialer { address, .. } => Some(address),
                                    libp2p::core::ConnectedPoint::Listener { send_back_addr, .. } => Some(send_back_addr),
                                };

                                if let Some(addr_val) = &addr {
                                    if !node_state.known_peers.contains(addr_val) {
                                        node_state.known_peers.push(addr_val.clone());
                                    }
                                }
                                // Check if we have pending requests
                                if !node_state.pending_manga_requests.is_empty() {
                                    tracing::info!("Sending pending requests to new peer {}", peer_id);
                                    for manga_id in node_state.pending_manga_requests.keys() {
                                        let req = manga_bay_p2p::protocol::AppRequest::GetManga { manga_id: manga_id.clone() };
                                        swarm.behaviour_mut().request_response.send_request(&peer_id, req);
                                    }
                                }
                                if !node_state.pending_chapters_requests.is_empty() {
                                    for manga_id in node_state.pending_chapters_requests.keys() {
                                        let req = manga_bay_p2p::protocol::AppRequest::GetChapters { manga_id: manga_id.clone() };
                                        swarm.behaviour_mut().request_response.send_request(&peer_id, req);
                                    }
                                }
                                if !node_state.pending_chapter_details_requests.is_empty() {
                                    for chapter_id in node_state.pending_chapter_details_requests.keys() {
                                        let req = manga_bay_p2p::protocol::AppRequest::GetChapterDetails { chapter_id: chapter_id.clone() };
                                        swarm.behaviour_mut().request_response.send_request(&peer_id, req);
                                    }
                                }
                                if !node_state.pending_block_requests.is_empty() {
                                    for hash in node_state.pending_block_requests.keys() {
                                        let req = manga_bay_p2p::protocol::AppRequest::GetBlock { block_hash: hash.clone() };
                                        swarm.behaviour_mut().request_response.send_request(&peer_id, req);
                                    }
                                }
                                if !node_state.pending_version_requests.is_empty() {
                                    for manga_id in node_state.pending_version_requests.keys() {
                                        let req = manga_bay_p2p::protocol::AppRequest::GetVersion { manga_id: manga_id.clone() };
                                        swarm.behaviour_mut().request_response.send_request(&peer_id, req);
                                    }
                                }

                                // Auto-discovery: Ask new peer for their known peers
                                tracing::info!("Asking new peer {} for their known peers", peer_id);
                                swarm.behaviour_mut().request_response.send_request(&peer_id, manga_bay_p2p::protocol::AppRequest::GetPeers);

                                // Enable Relay Listening if this is a bootstrap peer (or any peer really, but let's be safe)
                                // We construct the relay address: <PeerAddr>/p2p-circuit
                                if let Some(addr_val) = &addr {
                                    // Check if address already has p2p-circuit
                                    if !addr_val.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
                                        let relay_addr = addr_val.clone().with(Protocol::P2pCircuit);
                                        tracing::info!("Attempting to listen on relay address: {}", relay_addr);
                                        if let Err(e) = swarm.listen_on(relay_addr) {
                                            tracing::warn!("Failed to listen on relay address: {}", e);
                                        }
                                    }
                                }
                            }
                            libp2p::swarm::SwarmEvent::Behaviour(manga_bay_p2p::behaviour::MangaBehaviourEvent::Identify(
                                identify::Event::Received { peer_id, info, .. }
                            )) => {
                                tracing::info!("Received Identify from {}: protocols={:?}", peer_id, info.protocols);
                                for addr in info.listen_addrs {
                                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                                    if !node_state.known_peers.contains(&addr) {
                                        node_state.known_peers.push(addr);
                                    }
                                }
                            }
                            libp2p::swarm::SwarmEvent::Behaviour(manga_bay_p2p::behaviour::MangaBehaviourEvent::Mdns(
                                mdns::Event::Discovered(list)
                            )) => {
                                for (peer_id, multiaddr) in list {
                                    tracing::info!("mDNS discovered: {peer_id} at {multiaddr}");
                                    swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr.clone());
                                    if !node_state.known_peers.contains(&multiaddr) {
                                        node_state.known_peers.push(multiaddr);
                                    }
                                }
                            }
                            libp2p::swarm::SwarmEvent::Behaviour(manga_bay_p2p::behaviour::MangaBehaviourEvent::RequestResponse(
                                request_response::Event::Message { message, peer, .. }
                            )) => {
                                match message {
                                    request_response::Message::Request { request, channel, .. } => {
                                        match request {
                                            manga_bay_p2p::protocol::AppRequest::GetBlock { block_hash } => {
                                                tracing::info!("Received request for block: {}", block_hash);
                                                let data = match storage.get_chunk(&block_hash).await {
                                                    Ok(d) => d,
                                                    Err(e) => {
                                                        tracing::error!("Storage error: {:?}", e);
                                                        None
                                                    }
                                                };
                                                let size = data.as_ref().map(|d| d.len()).unwrap_or(0) as u64;
                                                let response = manga_bay_p2p::protocol::AppResponse::Block(block_hash.clone(), data);

                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                } else if size > 0 {
                                                    if let Err(e) = ratio_manager.record_upload(&peer.to_string(), size).await {
                                                        tracing::error!("Failed to record upload: {:?}", e);
                                                    }
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppRequest::GetManga { manga_id } => {
                                                tracing::info!("Received request for manga: {}", manga_id);
                                                let manga = match storage.get_manga(&manga_id).await {
                                                    Ok(m) => m,
                                                    Err(e) => {
                                                        tracing::error!("Storage error: {:?}", e);
                                                        None
                                                    }
                                                };
                                                let response = manga_bay_p2p::protocol::AppResponse::Manga(manga);
                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppRequest::GetChapters { manga_id } => {
                                                tracing::info!("Received request for chapters of manga: {}", manga_id);
                                                let chapters = match storage.list_chapters(&manga_id).await {
                                                    Ok(c) => c,
                                                    Err(e) => {
                                                        tracing::error!("Storage error: {:?}", e);
                                                        Vec::new()
                                                    }
                                                };
                                                let response = manga_bay_p2p::protocol::AppResponse::Chapters(manga_id, chapters);
                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppRequest::GetChapterDetails { chapter_id } => {
                                                tracing::info!("Received request for chapter details: {}", chapter_id);
                                                let details = match storage.get_chapter_details(&chapter_id).await {
                                                    Ok(d) => d,
                                                    Err(e) => {
                                                        tracing::error!("Storage error: {:?}", e);
                                                        None
                                                    }
                                                };
                                                let response = manga_bay_p2p::protocol::AppResponse::ChapterDetails(details);
                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppRequest::GetVersion { manga_id } => {
                                                tracing::info!("Received request for version of manga: {}", manga_id);
                                                let version = match storage.calculate_manga_version(&manga_id).await {
                                                    Ok(v) => v,
                                                    Err(e) => {
                                                        tracing::error!("Storage error: {:?}", e);
                                                        None
                                                    }
                                                };
                                                let response = manga_bay_p2p::protocol::AppResponse::Version(version);
                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppRequest::GetPeers => {
                                                tracing::info!("Received request for known peers");
                                                // Collect connected peers and known peers
                                                // Collect connected peers and known peers
                                                let peers: HashSet<String> = node_state.known_peers.iter().map(|a| a.to_string()).collect();
                                                // We could also add connected peers if we tracked their multiaddrs associated with PeerId

                                                // Just send what we have in known_peers
                                                let peer_list: Vec<String> = peers.into_iter().collect();
                                                let response = manga_bay_p2p::protocol::AppResponse::Peers(peer_list);
                                                if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                                    tracing::error!("Failed to send response: {:?}", e);
                                                }
                                            }
                                        }
                                    }
                                    request_response::Message::Response { response, .. } => {
                                        match response {
                                            manga_bay_p2p::protocol::AppResponse::Block(hash, data) => {
                                                let size = data.as_ref().map(|d| d.len()).unwrap_or(0) as u64;
                                                tracing::info!("Received block response for {}: {} bytes", hash, size);
                                                if size > 0 {
                                                    if let Err(e) = ratio_manager.record_download(&peer.to_string(), size).await {
                                                        tracing::error!("Failed to record download: {:?}", e);
                                                    }
                                                }
                                                if let Some(waiters) = node_state.pending_block_requests.remove(&hash) {
                                                    for tx in waiters {
                                                        let _ = tx.send(data.clone());
                                                    }
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppResponse::Manga(Some(manga)) => {
                                                tracing::info!("Received manga metadata: {}", manga.title);
                                                // Resolve pending requests
                                                if let Some(waiters) = node_state.pending_manga_requests.remove(&manga.id) {
                                                    for tx in waiters {
                                                        let _ = tx.send(Some(manga.clone()));
                                                    }
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppResponse::Manga(None) => {
                                                tracing::info!("Peer did not have the manga");
                                            }
                                            manga_bay_p2p::protocol::AppResponse::Chapters(manga_id, chapters) => {
                                                tracing::info!("Received {} chapters for {}", chapters.len(), manga_id);
                                                if let Some(waiters) = node_state.pending_chapters_requests.remove(&manga_id) {
                                                    for tx in waiters {
                                                        let _ = tx.send(chapters.clone());
                                                    }
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppResponse::ChapterDetails(Some(details)) => {
                                                tracing::info!("Received chapter details for {}", details.id);
                                                if let Some(waiters) = node_state.pending_chapter_details_requests.remove(&details.id) {
                                                    for tx in waiters {
                                                        let _ = tx.send(Some(details.clone()));
                                                    }
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppResponse::ChapterDetails(None) => {
                                                tracing::info!("Peer did not have the chapter details");
                                            }
                                            manga_bay_p2p::protocol::AppResponse::Version(Some(version)) => {
                                                tracing::info!("Received version for {}: {}", version.manga_id, version.hash);
                                                if let Some(waiters) = node_state.pending_version_requests.remove(&version.manga_id) {
                                                    for tx in waiters {
                                                        let _ = tx.send(vec![version.clone()]);
                                                    }
                                                }
                                            }
                                            manga_bay_p2p::protocol::AppResponse::Version(None) => {
                                                tracing::info!("Peer did not have the manga version");
                                            }
                                            manga_bay_p2p::protocol::AppResponse::Peers(peers) => {
                                                tracing::info!("Received {} peers from discovery", peers.len());
                                                for p in peers {
                                                    if let Ok(addr) = p.parse::<Multiaddr>() {
                                                        if !node_state.known_peers.contains(&addr) {
                                                            tracing::info!("Discovered new peer: {}", addr);
                                                            node_state.known_peers.push(addr.clone());
                                                            // Also add to Kademlia if we had the peer_id, but here we only have multiaddr.
                                                            // We can try to dial it.
                                                            if swarm.dial(addr).is_ok() {
                                                                tracing::info!("Dialing discovered peer...");
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            libp2p::swarm::SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                                tracing::debug!("Failed to dial {:?}: {:?}", peer_id, error);
                            }
                            libp2p::swarm::SwarmEvent::IncomingConnectionError { local_addr, send_back_addr, error, .. } => {
                                tracing::debug!("Incoming connection failed from {} to {}: {:?}", send_back_addr, local_addr, error);
                            }
                            event => tracing::debug!("Swarm Event: {:?}", event),
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
        let bytes = tokio::fs::read(&key_path).await?;
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

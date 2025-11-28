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
}

#[derive(Debug)]
pub enum NodeCommand {
    GetNodeInfo(oneshot::Sender<NodeInfo>),
    ConnectPeer(Multiaddr, oneshot::Sender<Result<()>>),
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
                                let info = NodeInfo {
                                    peer_id: local_peer_id.to_string(),
                                    listeners,
                                };
                                let _ = reply.send(info);
                            }
                            NodeCommand::ConnectPeer(addr, reply) => {
                                let result = swarm.dial(addr).map_err(|e| anyhow::anyhow!(e));
                                let _ = reply.send(result);
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
                                        tracing::info!("Received request for block: {}", request.block_hash);

                                        // Check Ratio (Enforcement)
                                        // In a real scenario, we would check the requester's ratio here.
                                        // For now, we just log it.

                                        let data = match storage.get_chunk(&request.block_hash).await {
                                            Ok(d) => d,
                                            Err(e) => {
                                                tracing::error!("Storage error: {:?}", e);
                                                None
                                            }
                                        };

                                        let size = data.as_ref().map(|d| d.len()).unwrap_or(0) as u64;
                                        let response = crate::p2p::protocol::BlockResponse { data };

                                        if let Err(e) = swarm.behaviour_mut().request_response.send_response(channel, response) {
                                            tracing::error!("Failed to send response: {:?}", e);
                                        } else if size > 0 {
                                            // Record Upload
                                            if let Err(e) = ratio_manager.record_upload(&peer.to_string(), size).await {
                                                tracing::error!("Failed to record upload: {:?}", e);
                                            }
                                        }
                                    }
                                    request_response::Message::Response { response, .. } => {
                                        let size = response.data.as_ref().map(|d| d.len()).unwrap_or(0) as u64;
                                        tracing::info!("Received block response: {} bytes", size);

                                        if size > 0 {
                                            // Record Download
                                            if let Err(e) = ratio_manager.record_download(&peer.to_string(), size).await {
                                                tracing::error!("Failed to record download: {:?}", e);
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

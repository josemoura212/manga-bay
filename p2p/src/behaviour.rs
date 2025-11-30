use libp2p::{
    dcutr, gossipsub, identify, kad, mdns, relay, request_response, swarm::NetworkBehaviour,
};

#[derive(NetworkBehaviour)]
pub struct MangaBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
    pub mdns: libp2p::swarm::behaviour::toggle::Toggle<mdns::tokio::Behaviour>,
    pub request_response: request_response::cbor::Behaviour<
        crate::protocol::AppRequest,
        crate::protocol::AppResponse,
    >,
    pub relay: relay::client::Behaviour,
    pub relay_server: relay::Behaviour,
    pub dcutr: dcutr::Behaviour,
}

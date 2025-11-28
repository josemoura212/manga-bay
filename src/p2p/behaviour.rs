use libp2p::{gossipsub, identify, kad, mdns, request_response, swarm::NetworkBehaviour};

#[derive(NetworkBehaviour)]
pub struct MangaBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub request_response: request_response::cbor::Behaviour<
        crate::p2p::protocol::BlockRequest,
        crate::p2p::protocol::BlockResponse,
    >,
}

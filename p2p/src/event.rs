use libp2p::{gossipsub, identify, kad, mdns};

#[derive(Debug)]
pub enum NetworkEvent {
    Gossipsub(gossipsub::Event),
    Kademlia(kad::Event),
    Identify(identify::Event),
    Mdns(mdns::Event),
}

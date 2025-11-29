use libp2p::{core::upgrade, noise, tcp, yamux, Transport};
use std::time::Duration;

pub fn build_transport(
    keypair: &libp2p::identity::Keypair,
) -> std::io::Result<
    libp2p::core::transport::Boxed<(libp2p::PeerId, libp2p::core::muxing::StreamMuxerBox)>,
> {
    let tcp_transport = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true));

    let noise_config = noise::Config::new(keypair).unwrap();
    let yamux_config = yamux::Config::default();

    Ok(tcp_transport
        .upgrade(upgrade::Version::V1)
        .authenticate(noise_config)
        .multiplex(yamux_config)
        .timeout(Duration::from_secs(20))
        .boxed())
}

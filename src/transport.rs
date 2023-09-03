
use libp2p::core::transport::timeout::TransportTimeout;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::{muxing::StreamMuxerBox, transport::OrTransport,transport::Boxed};
use libp2p::dns::TokioDnsConfig;
use libp2p::identity;
use libp2p::quic as quic;
use async_std::io;
use futures::future::Either;
use libp2p::relay::client::Transport as ClientTransport;
use libp2p::tcp::{tokio::Transport as TokioTcpTransport, Config as GenTcpConfig};
use libp2p::yamux;
use libp2p::{PeerId, Transport};

use std::time::Duration;

pub fn build_transport(
    keypair: identity::Keypair,
    relay: Option<ClientTransport>,
) -> io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
    let local_key = identity::Keypair::generate_ed25519();

    let yamux_config = {
        let mut config = yamux::Config::default();
        config.set_max_buffer_size(16 * 1024 * 1024);
        config.set_receive_window_size(16 * 1024 * 1024);
         config
    };

    let tcp_transport =
        TokioTcpTransport::new(GenTcpConfig::default().nodelay(true).port_reuse(true));

    let transport_timeout = TransportTimeout::new(tcp_transport, Duration::from_secs(30));
    let transport = TokioDnsConfig::system(transport_timeout)?;

    let transport = match relay {
        Some(relay) => {
            let transport = OrTransport::new(relay, transport);
            transport
                .upgrade(Version::V1)
                .authenticate(libp2p_noise::Config::new(&local_key).unwrap())
                .multiplex(yamux_config)
                .timeout(Duration::from_secs(20))
                .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
                .boxed()
        }
        None => transport
            .upgrade(Version::V1)
            .authenticate(libp2p_noise::Config::new(&local_key).unwrap())
            .multiplex(yamux_config)
            .timeout(Duration::from_secs(20))
            .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
            .boxed(),
    };
 
    let quic_transport = quic::async_std::Transport::new(quic::Config::new(&keypair));
    let transport = OrTransport::new(quic_transport, transport)
        .map(|either_output, _| match either_output {
            Either::Left((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
            Either::Right((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
        })
        .boxed();

    Ok(transport)
}

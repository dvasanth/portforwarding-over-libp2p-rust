#![allow(unused_imports)]
use std::sync::{atomic::AtomicBool, Arc};

use crypto_seal::key::PrivateKey;
use libp2p::gossipsub::{self, Behaviour as Gossipsub, MessageAuthenticity};
use libp2p::relay::client::Transport as ClientTransport;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{identity::Keypair,gossipsub::Event as GossipsubEvent, swarm::behaviour::toggle::Toggle, PeerId, Swarm};

use libp2p::{
    self,
    autonat::{Behaviour as Autonat, Event as AutonatEvent},
    dcutr::{Behaviour as DcutrBehaviour, Event as DcutrEvent},
    identify::{Behaviour as Identify, Config as IdentifyConfig, Event as IdentifyEvent},
    kad::{store::MemoryStore, Kademlia, KademliaConfig, KademliaEvent},
    mdns::{tokio::Behaviour as Mdns, Config as MdnsConfig, Event as MdnsEvent},
    ping::{Behaviour as Ping, Event as PingEvent},
    relay::client::{self, Behaviour as RelayClient, Event as RelayClientEvent},
    relay::{Behaviour as RelayServer, Event as RelayServerEvent},
    rendezvous::{
        self,
        client::{Behaviour as Rendezvous, Event as RendezvousEvent},
    },
    dcutr,
};

use libp2p_helper::gossipsub::GossipsubStream;
use tokio::io::{self, AsyncBufReadExt};
use std::time::Duration;

use crate::transport;

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "PortForwardingBehaviourEvent", event_process = false)]
pub struct PortForwardingBehaviour {
    pub relay_client: Toggle<RelayClient>,
    pub relay_server: Toggle<RelayServer>,
    pub dcutr: dcutr::Behaviour,
    pub autonat: Autonat,
    pub kademlia: Kademlia<MemoryStore>,
    pub identify: Identify,
    pub ping: Ping,
    pub rendezvous: Rendezvous,
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: Toggle<Mdns>,
}

pub enum PortForwardingBehaviourEvent {
    RelayClient(RelayClientEvent),
    RelayServer(RelayServerEvent),
    Dcutr(DcutrEvent),
    Autonat(AutonatEvent),
    Kad(KademliaEvent),
    Identify(IdentifyEvent),
    Ping(PingEvent),
    Gossipsub(gossipsub::Event),
    Rendezvous(RendezvousEvent),
    Mdns(MdnsEvent),
}

impl From<gossipsub::Event> for PortForwardingBehaviourEvent {
    fn from(event: gossipsub::Event ) -> Self {
        // Convert GossipsubStream event to PortForwardingBehaviourEvent
        PortForwardingBehaviourEvent::Gossipsub(event)
    }
}


impl From<RelayClientEvent> for PortForwardingBehaviourEvent {
    fn from(event: RelayClientEvent) -> Self {
        PortForwardingBehaviourEvent::RelayClient(event)
    }
}

impl From<RelayServerEvent> for PortForwardingBehaviourEvent {
    fn from(event: RelayServerEvent) -> Self {
        PortForwardingBehaviourEvent::RelayServer(event)
    }
}

impl From<DcutrEvent> for PortForwardingBehaviourEvent {
    fn from(event: DcutrEvent) -> Self {
        PortForwardingBehaviourEvent::Dcutr(event)
    }
}

impl From<AutonatEvent> for PortForwardingBehaviourEvent {
    fn from(event: AutonatEvent) -> Self {
        PortForwardingBehaviourEvent::Autonat(event)
    }
}

impl From<KademliaEvent> for PortForwardingBehaviourEvent {
    fn from(event: KademliaEvent) -> Self {
        PortForwardingBehaviourEvent::Kad(event)
    }
}

impl From<IdentifyEvent> for PortForwardingBehaviourEvent {
    fn from(event: IdentifyEvent) -> Self {
        PortForwardingBehaviourEvent::Identify(event)
    }
}

impl From<MdnsEvent> for PortForwardingBehaviourEvent {
    fn from(event: MdnsEvent) -> Self {
        PortForwardingBehaviourEvent::Mdns(event)
    }
}

impl From<PingEvent> for PortForwardingBehaviourEvent {
    fn from(event: PingEvent) -> Self {
        PortForwardingBehaviourEvent::Ping(event)
    }
}

impl From<RendezvousEvent> for PortForwardingBehaviourEvent {
    fn from(event: RendezvousEvent) -> Self {
        PortForwardingBehaviourEvent::Rendezvous(event)
    }
}

impl PortForwardingBehaviour {
    pub async fn create_behaviour(keypair: Keypair) -> anyhow::Result<Self> {
        let peer_id = keypair.public().to_peer_id();

        let mdns = Some(Mdns::new(MdnsConfig::default(), peer_id)?).into();
        let autonat = Autonat::new(peer_id, Default::default());
        let ping = Ping::default();
        let identify = Identify::new(
            IdentifyConfig::new("/ipfs/0.1.0".into(), keypair.public())
                .with_agent_version("libp2p-portforwarding".into()),
        );
        let store = MemoryStore::new(peer_id);

        let mut kad_config = KademliaConfig::default();
        kad_config.set_query_timeout(Duration::from_secs(5 * 60));
        let kademlia = Kademlia::with_config(peer_id, store, kad_config);
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .max_transmit_size(262144)
            .build()
            .expect("valid config");        
        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        ).expect("Valid configuration");
        let rendezvous = Rendezvous::new(keypair);
        Ok(Self {
            mdns,
            relay_client: None.into(),
            relay_server: None.into(),
            dcutr:dcutr::Behaviour::new(peer_id),
            autonat,
            kademlia,
            identify,
            ping,
            gossipsub,
            rendezvous,
        })
    }

    pub fn create_swarm(
        self,
        keypair: Keypair,
        relay_transport: Option<ClientTransport>,
    ) -> anyhow::Result<Swarm<Self>> {
        let peerid = keypair.public().to_peer_id();
        let transport = transport::build_transport(keypair, relay_transport)?;

        let swarm = libp2p::swarm::SwarmBuilder::with_tokio_executor(transport, self, peerid)
            .dial_concurrency_factor(10_u8.try_into().unwrap())
            .build();
        Ok(swarm)
    }
}

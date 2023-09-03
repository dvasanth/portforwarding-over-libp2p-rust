mod behaviour;
mod transport;
mod proxyserver;
mod listener;
mod forwarder;
use std::collections::HashMap;
use behaviour::{PortForwardingBehaviour, PortForwardingBehaviourEvent};
use clap::Parser;
use forwarder::PFForwarder;
use futures::StreamExt;
use libp2p_identity as identity;
use libp2p::{
    identify::{Event as IdentifyEvent, Info as IdentifyInfo},
    kad::{record::Key, GetProvidersOk, KademliaEvent, QueryId, QueryResult},
    mdns::Event as MdnsEvent,
    swarm::SwarmEvent,
    Multiaddr, PeerId, Swarm,
    gossipsub
};
use listener::PFListener;
use uuid::Uuid;
use std::{collections::HashSet, hash::Hash, str::FromStr, time::Duration};
use std::sync::Arc;
use bincode::{serialize, deserialize};
use std::net::SocketAddr;
use tokio::sync::mpsc;
use env_logger;

#[derive(Hash, PartialEq, Eq)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub address: Vec<Multiaddr>,
    pub public_key: identity::PublicKey,
}

#[derive(Debug, Parser)]
#[clap(name = "libp2p portforwarding")]
struct CliOpt {
    #[clap(long)]
    forwarder: bool,

    #[clap(long)]
    topic: Option<String>,

    #[clap(long)]
    forward_p2p_address: Option<String>,
}


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let opt = CliOpt::parse();
    let kp = identity::Keypair::generate_ed25519();
    let  behaviour = PortForwardingBehaviour::create_behaviour(kp.clone()).await?;
    let forward_address = SocketAddr::from(([127, 0, 0, 1], 8090));
    let listen_address = SocketAddr::from(([127, 0, 0, 1], 8080));

    if opt.forwarder == true {
        let proxy = proxyserver::HttpProxy::new(forward_address);

            tokio::spawn(async move {
                if let Err(err) = proxy.run().await {
                    eprintln!("HTTP proxy error: {:?}", err);
                }
                
            });
    } else{
        // Run a proxy forward on listener side  
        println!( "Running proxy server on {} Set this address on browser/curl", listen_address);
    }

    let (listener_client_tx, mut listener_client_rx) = mpsc::unbounded_channel::<(u64, Vec<u8>)>(); 
    let (forwarder_client_tx, mut forwarder_client_rx) = mpsc::unbounded_channel::<(u64, Vec<u8>)>(); 

    let pf_listener = Arc::new(listener::PFListener::new(listener_client_tx, listen_address));
    let pf_forwarder = Arc::new(forwarder::PFForwarder::new(forwarder_client_tx, forward_address));
    if opt.forwarder == false {
        listener::PFListener::start(pf_listener.clone());
    }

    let mut swarm = behaviour.create_swarm(kp.clone(), None)?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap())?;
    swarm.listen_on("/ip6/::/tcp/0".parse().unwrap())?;
    swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())?;


    // bootstrap address from ipfs/kubo
    let new_addr = Multiaddr::from_str ("/ip4/104.131.131.82/tcp/4001")?;
    swarm
    .behaviour_mut()
    .kademlia
    .add_address(&PeerId::from_str("QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ")?, new_addr);
    
     let topic: String = if opt.forwarder {
        Uuid::new_v4().to_string()
    } else {
        opt.topic.unwrap_or_default()
    };
 
    // Create a Gosspipsub topic
    let gossipsub_topic = gossipsub::IdentTopic::new(topic.clone());
    swarm.behaviour_mut().gossipsub.subscribe(&gossipsub_topic)?;


    let topic_key = Key::new(&topic);

    let mut query_registry = HashSet::new();

    if opt.forwarder == true {
        println!("Topic started : {}", topic);
        query_registry.insert(
            swarm
                .behaviour_mut()
                .kademlia
                .start_providing(topic_key.clone())?,
        );
    } else {
        println!("Joining Topic : {}", topic);
    }

    let mut peer_book: HashSet<PeerInfo> = HashSet::new();
    let mut bootstrap_interval = tokio::time::interval(Duration::from_secs(120));
    let mut get_provider_interval = tokio::time::interval(Duration::from_secs(2));
    let mut peer_list: HashSet<PeerId> = HashSet::new();
    let mut lookup_active = false;
    let mut sender_sequence_id:HashMap<u64, u64> =  HashMap::new(); //connection id to sequence number
    let mut receiver_sequence_id:HashMap<u64, u64> =  HashMap::new(); //connection id to sequence number
    let mut received_messages:HashMap<(u64, u64),  Vec<u8>> =  HashMap::new();
    let mut observer_address_count:i32 = 0;
    loop {
        tokio::select! {
          // Relay the forwarder data to listener
            Some((client_id, data)) = forwarder_client_rx.recv() =>{
                let tuple_message: (u64, u64, Vec<u8>) = (*sender_sequence_id.entry(client_id).or_insert(1), client_id, data);

                // Serialize the tuple message into a byte array
                let serialized_message = serialize(&tuple_message)?;
                //println!( "Forwarder sending message len: {}", tuple_message.2.len());
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(gossipsub_topic.clone(), serialized_message.clone()) {
                    println!( "Error sending message: {}", e);
                    continue;
                }
                *sender_sequence_id.get_mut(&client_id).unwrap()+=1
            }

            // Relay the listener data  to forwarder 
            Some((client_id, data)) = listener_client_rx.recv() =>{
                let tuple_message: (u64, u64, Vec<u8>) = (*sender_sequence_id.entry(client_id).or_insert(1), client_id, data);

                // Serialize the tuple message into a byte array
                let serialized_message = serialize(&tuple_message)?;
                //println!( "Listener sending message len: {}", tuple_message.2.len());
                if let Err(e) = swarm.behaviour_mut().gossipsub.publish(gossipsub_topic.clone(), serialized_message.clone()) {
                    println!( "Error sending message: {}", e);
                    continue;
                }
                *sender_sequence_id.get_mut(&client_id).unwrap()+=1
            }

            event = swarm.select_next_some() => {
                match swarm_event( &mut swarm, event, &mut peer_book, &mut peer_list, &mut query_registry, &mut lookup_active, opt.forwarder, pf_forwarder.clone(), pf_listener.clone(), &mut receiver_sequence_id, &mut received_messages, &mut observer_address_count).await {
                    Ok(_) => {},
                    Err(e) => {
                        println!("Error processing event: {}", e);
                    }
                }
            }
            _ = bootstrap_interval.tick() => {
               // swarm.behaviour_mut().kademlia.bootstrap()?;
                if opt.forwarder == true {
                    query_registry.insert(
                        swarm
                            .behaviour_mut()
                            .kademlia
                            .start_providing(topic_key.clone())?,
                );
            }
            },
            _ = get_provider_interval.tick() => {
                // forwarder doesn't need to search for provider
                if opt.forwarder == false && !lookup_active {
                    query_registry.insert(swarm.behaviour_mut().kademlia.get_providers(topic_key.clone()));
                    lookup_active = true;
                }
            },
        }
    }

}

async fn swarm_event<S>(
    swarm: &mut Swarm<PortForwardingBehaviour>,
    event: SwarmEvent<PortForwardingBehaviourEvent, S>,
    peer_book: &mut HashSet<PeerInfo>,
    _peer_list: &mut HashSet<PeerId>,
    query_registry: &mut HashSet<QueryId>,
    lookup_active: &mut bool,
    forwarder: bool,
    pf_forwarder: Arc<PFForwarder>,
    pf_listener: Arc<PFListener>,
    receiver_sequence_id:&mut HashMap<u64, u64>,
    received_messages:&mut HashMap<(u64, u64),  Vec<u8>>,
    observed_address_count:&mut i32
) -> anyhow::Result<()> {
    match event {
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Rendezvous(_)) => {}
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::RelayClient(_event)) => {
            // writeln!(stdout, "Relay Client Event: {:?}", event)?;
        }
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::RelayServer(_event)) => {}
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Gossipsub(
            libp2p::gossipsub::Event::Unsubscribed { peer_id, .. },
        )) => {
            for peer in peer_book.iter() {
                if peer.peer_id == peer_id {
                    println!("{} has left.", peer.public_key.to_peer_id());
                    break;
                }
            }
        }
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Gossipsub(gossipsub::Event::Message {
            propagation_source: _,
            message_id: _,
            message,
        })) => {
                let received_message: Result<(u64, u64, Vec<u8>), _> = deserialize(&message.data);

                match received_message {
                    Ok(tuple_message) => {
                        let seqid = tuple_message.0;
                        let clientid = tuple_message.1;
                        received_messages.insert((clientid, seqid),  tuple_message.2);
                        while let Some(data) = received_messages.remove(&(clientid, *receiver_sequence_id.entry(clientid).or_insert(1))) {
                            if forwarder == true {
                                // Send the received message to local server thro pf forwarder
                                //let _ = println!("Sending to forwarder tuple message len: {}",  seq_tuple_message.1.len());
                                let _ =pf_forwarder.send(clientid,data).await;
                                
                            }else{
                                //let _ = println!("Sending to listener tuple message len: {}",  seq_tuple_message.1.len());
                                // Send the received message to local client thro pf listeners
                                let _ = pf_listener.send(clientid,data);
                            }
                            *receiver_sequence_id.get_mut(&clientid).unwrap()+=1;
                        }
                    }
                    Err(err) => {
                        let _ = println!( "Error deserializing tuple message: {}", err);
                    }
                }
        }
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Mdns(event)) => match event {
            MdnsEvent::Discovered(list) => {
                for (peer, _) in list {
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer);
                }
            }
            MdnsEvent::Expired(list) => {
                for (peer, _) in list {
                    if let Some(mdns) = swarm.behaviour().mdns.as_ref() {
                        if !mdns.has_node(&peer) {
                            swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer);
                        }
                    }
                }
            }
        },
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Ping(_event)) => {}
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Identify(event)) => {
            if let IdentifyEvent::Received {
                peer_id,
                info:
                    IdentifyInfo {
                        listen_addrs,
                        protocols,
                        public_key,
                        agent_version,
                        observed_addr,
                        ..
                    },
            } = event
            {
                //println!("External address:  {}", observed_addr.clone());
                if  *observed_address_count < 6 {
                    swarm.add_external_address(observed_addr);
                    *observed_address_count+=1;
                }
                if agent_version == "libp2p-portforwarding" {
                    println!("Found other peer, proxy active :  {}", peer_id);
                    peer_book.insert(PeerInfo {
                        peer_id,
                        public_key,
                        address: listen_addrs.clone(),
                    });
                }


                if protocols
                    .iter()
                    .any(|p| *p == libp2p::autonat::DEFAULT_PROTOCOL_NAME)
                {
                    for addr in listen_addrs {
                        swarm
                            .behaviour_mut()
                            .autonat
                            .add_server(peer_id, Some(addr));
                    }
                }
            }
        }
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Kad(
            KademliaEvent::OutboundQueryProgressed { id, result, step, .. },
        )) => match result {
            QueryResult::StartProviding(Err(err)) => {
                eprintln!("Failed to put provider record: {err:?}");
            }            
            QueryResult::GetProviders(Ok(GetProvidersOk::FinishedWithNoAdditionalRecord {
                ..
            })) => {
                if step.last && query_registry.remove(&id) {
                    *lookup_active = false;
                }
            }
            _ => {}
        },
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Kad(_)) => {}
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Autonat(_)) => {}
        SwarmEvent::Behaviour(PortForwardingBehaviourEvent::Dcutr(_)) => {}
        SwarmEvent::ConnectionEstablished { .. } => {}
        SwarmEvent::ConnectionClosed { .. } => {}
        SwarmEvent::IncomingConnection { .. } => {}
        SwarmEvent::IncomingConnectionError { .. } => {}
        SwarmEvent::OutgoingConnectionError { .. } => {}
        SwarmEvent::NewListenAddr { .. } => {}
        SwarmEvent::ExpiredListenAddr { .. } => {}
        SwarmEvent::ListenerClosed { .. } => {}
        SwarmEvent::ListenerError { .. } => {}
        _ => {}
    }
    Ok(())
}



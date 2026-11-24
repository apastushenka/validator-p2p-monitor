//! The monitor's side of the chain's Kademlia DHT.

use std::str::FromStr;

use anyhow::bail;
use libp2p::{
    PeerId, StreamProtocol, Swarm, SwarmBuilder,
    futures::StreamExt,
    kad::{self, Event::OutboundQueryProgressed, QueryStats, store::MemoryStore},
    multiaddr::{Multiaddr, Protocol},
    noise,
    swarm::SwarmEvent,
    tcp, yamux,
};

/// Substrate nodes list many addresses for each peer they return, and their answers overflow
/// the 16 KiB that libp2p allows by default: every request then fails.
const MAX_PACKET_SIZE: usize = 1024 * 1024;

/// A node to enter the DHT through.
#[derive(Clone)]
pub struct Bootnode {
    pub peer_id: PeerId,
    /// The address without its `/p2p` part: Kademlia takes the peer id separately.
    pub address: Multiaddr,
}

impl FromStr for Bootnode {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let mut address: Multiaddr = input.parse().map_err(|err| format!("{err}"))?;

        match address.pop() {
            Some(Protocol::P2p(peer_id)) if !address.is_empty() => Ok(Self { peer_id, address }),
            _ => Err("expected an address that ends in /p2p/<peer id>".to_string()),
        }
    }
}

/// Enter the DHT as a client, which neither stores records nor answers queries.
pub async fn join(
    genesis_hash: [u8; 32],
    bootnodes: &[Bootnode],
) -> anyhow::Result<Swarm<kad::Behaviour<MemoryStore>>> {
    let mut swarm = SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_dns()?
        // A node listens on WebSocket instead of plain TCP unless it is a validator or sets
        // --listen-addr, and a validator may set a WebSocket address there too.
        .with_websocket(noise::Config::new, yamux::Config::default)
        .await?
        .with_behaviour(|key| kademlia(key.public().to_peer_id(), genesis_hash, bootnodes))?
        .build();

    // Not a full bootstrap: its first stage is this lookup of our own peer id, and the later stages
    // only fill the routing table, which lookups don't need, as each walks from the boot nodes to
    // the peers closest to its key. Any answer shows that a boot node speaks this chain's protocol.
    let local_peer_id = *swarm.local_peer_id();
    let stats = lookup_closest_peers(&mut swarm, local_peer_id).await;
    if stats.num_successes() == 0 {
        let protocol = &swarm.behaviour().protocol_names()[0];
        bail!(
            "no boot node answered on {protocol}: they are unreachable or belong to another chain"
        );
    }

    Ok(swarm)
}

/// A Kademlia client that knows only the boot nodes.
fn kademlia(
    peer_id: PeerId,
    genesis_hash: [u8; 32],
    bootnodes: &[Bootnode],
) -> kad::Behaviour<MemoryStore> {
    let protocol = StreamProtocol::try_from_owned(format!("/{}/kad", hex::encode(genesis_hash)))
        .expect("the name starts with a slash");

    let mut config = kad::Config::new(protocol);
    config.set_max_packet_size(MAX_PACKET_SIZE);

    let mut kademlia = kad::Behaviour::with_config(peer_id, MemoryStore::new(peer_id), config);
    kademlia.set_mode(Some(kad::Mode::Client));
    for bootnode in bootnodes {
        kademlia.add_address(&bootnode.peer_id, bootnode.address.clone());
    }

    kademlia
}

/// Look up the peers closest to the key and return the lookup's stats once it ends.
///
/// The lookup reports only once, when it ends, so its first event is the result.
async fn lookup_closest_peers(
    swarm: &mut Swarm<kad::Behaviour<MemoryStore>>,
    key: PeerId,
) -> QueryStats {
    let query = swarm.behaviour_mut().get_closest_peers(key);
    loop {
        if let SwarmEvent::Behaviour(OutboundQueryProgressed { id, stats, .. }) =
            swarm.select_next_some().await
            && id == query
        {
            return stats;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Bootnode;

    const PEER_ID: &str = "12D3KooWSz8r2WyCdsfWHgPyvD8GKQdJ1UAiRmrcrs8sQB3fe2KU";

    #[test]
    fn bootnode_splits_peer_id_from_address() {
        let bootnode: Bootnode =
            format!("/dns/polkadot-bootnode-0.polkadot.io/tcp/30333/p2p/{PEER_ID}")
                .parse()
                .unwrap();

        assert_eq!(bootnode.peer_id.to_string(), PEER_ID);
        assert_eq!(
            bootnode.address.to_string(),
            "/dns/polkadot-bootnode-0.polkadot.io/tcp/30333"
        );
    }

    #[test]
    fn bootnode_without_peer_id_is_rejected() {
        assert!(
            "/dns/polkadot-bootnode-0.polkadot.io/tcp/30333"
                .parse::<Bootnode>()
                .is_err()
        );
    }

    #[test]
    fn bootnode_with_peer_id_only_is_rejected() {
        assert!(format!("/p2p/{PEER_ID}").parse::<Bootnode>().is_err());
    }
}

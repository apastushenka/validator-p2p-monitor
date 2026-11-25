//! The monitor's side of the chain's Kademlia DHT.

use std::str::FromStr;

use libp2p::{
    PeerId, StreamProtocol,
    futures::StreamExt,
    kad::{self, Event::OutboundQueryProgressed, QueryStats, store::MemoryStore},
    multiaddr::{Multiaddr, Protocol},
    swarm::SwarmEvent,
};

use super::{BehaviourEvent::Kademlia, Network};

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

/// A Kademlia client that knows only the boot nodes.
pub fn kademlia(
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

impl Network {
    /// Look up the peers closest to the key and return the lookup's stats once it ends.
    ///
    /// The lookup reports only once, when it ends, so its first event is the result.
    pub(super) async fn lookup_closest_peers(&mut self, key: PeerId) -> QueryStats {
        let query = self.swarm.behaviour_mut().kademlia.get_closest_peers(key);
        loop {
            if let SwarmEvent::Behaviour(Kademlia(OutboundQueryProgressed { id, stats, .. })) =
                self.swarm.select_next_some().await
                && id == query
            {
                return stats;
            }
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

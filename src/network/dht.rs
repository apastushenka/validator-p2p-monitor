//! The monitor's side of the chain's Kademlia DHT.

use std::str::FromStr;

use anyhow::{Context, bail};
use libp2p::{
    PeerId, StreamProtocol,
    kad::{self, store::MemoryStore},
    multiaddr::{Multiaddr, Protocol},
};
use tokio::sync::mpsc;

use super::{Command, Network, Progress, STOPPED};

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

impl Network {
    /// Enter the DHT through the boot nodes.
    pub async fn join(&self) -> anyhow::Result<()> {
        // Not a full bootstrap: only its first stage, the lookup of our own peer id, as Kademlia
        // runs the whole of it in the background by itself. Any answer shows that a boot node
        // speaks this chain's protocol.
        let (progress, mut events) = mpsc::unbounded_channel();

        let cmd = Command::GetClosestPeers {
            key: self.local_peer_id,
            progress,
        };
        self.send(cmd).await?;

        // The lookup reports only once, when it ends, so its first event is the result.
        let Progress { stats } = events.recv().await.context(STOPPED)?;
        if stats.num_successes() == 0 {
            bail!(
                "no boot node answered on this chain's Kademlia protocol: they are unreachable or \
                 belong to another chain"
            );
        }

        Ok(())
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

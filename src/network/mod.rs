//! The monitor's node in the chain's p2p network.
//!
//! One swarm does all of it, as libp2p intends: a swarm is a node with one peer id, so each
//! protocol becomes a field of [`Behaviour`] rather than a swarm of its own.

mod dht;

use anyhow::bail;
use libp2p::{
    Swarm, SwarmBuilder,
    kad::{self, store::MemoryStore},
    noise,
    swarm::NetworkBehaviour,
    tcp, yamux,
};

pub use self::dht::Bootnode;

pub struct Network {
    swarm: Swarm<Behaviour>,
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    kademlia: kad::Behaviour<MemoryStore>,
}

impl Network {
    /// Enter the DHT as a client, which neither stores records nor answers queries.
    pub async fn join(genesis_hash: [u8; 32], bootnodes: &[Bootnode]) -> anyhow::Result<Self> {
        let swarm = SwarmBuilder::with_new_identity()
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
            .with_behaviour(|key| Behaviour {
                kademlia: dht::kademlia(key.public().to_peer_id(), genesis_hash, bootnodes),
            })?
            .build();
        let mut network = Self { swarm };

        // Not a full bootstrap: its first stage is this lookup of our own peer id, and the later
        // stages only fill the routing table, which lookups don't need, as each walks from the boot
        // nodes to the peers closest to its key. Any answer shows that a boot node speaks this
        // chain's protocol.
        let local_peer_id = *network.swarm.local_peer_id();
        let stats = network.lookup_closest_peers(local_peer_id).await;
        if stats.num_successes() == 0 {
            let protocol = &network.swarm.behaviour().kademlia.protocol_names()[0];
            bail!(
                "no boot node answered on {protocol}: they are unreachable or belong to another chain"
            );
        }

        Ok(network)
    }
}

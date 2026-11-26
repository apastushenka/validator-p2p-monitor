//! The monitor's node in the chain's p2p network.
//!
//! One swarm does all of it, as libp2p intends: a swarm is a node with one peer id, so each
//! protocol becomes a field of [`Behaviour`] rather than a swarm of its own. [`EventLoop`] owns
//! the swarm and runs for as long as the monitor does; the rest of the monitor reaches it
//! through [`Network`], one command per network operation.

mod dht;
mod event_loop;

use anyhow::Context;
use libp2p::{
    PeerId, Swarm, SwarmBuilder,
    kad::{self, QueryResult, QueryStats, RecordKey, store::MemoryStore},
    noise,
    swarm::NetworkBehaviour,
    tcp, yamux,
};
use tokio::sync::mpsc;

pub use self::dht::{Bootnode, RecordLookup};
use self::event_loop::EventLoop;

const STOPPED: &str = "the network event loop stopped";

pub struct Network {
    commands: mpsc::Sender<Command>,
    local_peer_id: PeerId,
}

impl Network {
    /// Start the monitor's node in the background, as a DHT client, which neither stores records
    /// nor answers queries.
    pub async fn new(genesis_hash: [u8; 32], bootnodes: &[Bootnode]) -> anyhow::Result<Self> {
        let swarm = swarm(genesis_hash, bootnodes).await?;
        let local_peer_id = *swarm.local_peer_id();

        let (commands, receiver) = mpsc::channel(1);
        tokio::spawn(EventLoop::new(swarm, receiver).run());

        Ok(Self { commands, local_peer_id })
    }

    async fn send(&self, command: Command) -> anyhow::Result<()> {
        self.commands.send(command).await.ok().context(STOPPED)
    }
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    kademlia: kad::Behaviour<MemoryStore>,
}

/// Each command that starts a query carries the channel that the query's events go to. The
/// channel is unbounded, as the event loop cannot wait for room in it, but a query sends no more
/// events than the peers it asks, plus its end.
enum Command {
    GetClosestPeers {
        key: PeerId,
        progress: mpsc::UnboundedSender<Progress>,
    },
    GetRecord {
        key: RecordKey,
        progress: mpsc::UnboundedSender<Progress>,
    },
}

/// One event of a running query, as the event loop forwards it to whoever started the query.
struct Progress {
    result: QueryResult,
    stats: QueryStats,
    last: bool,
}

async fn swarm(genesis_hash: [u8; 32], bootnodes: &[Bootnode]) -> anyhow::Result<Swarm<Behaviour>> {
    Ok(SwarmBuilder::with_new_identity()
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
        .build())
}

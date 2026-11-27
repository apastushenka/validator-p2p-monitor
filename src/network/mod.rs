//! The monitor's node in the chain's p2p network.
//!
//! One swarm does all of it, as libp2p intends: a swarm is a node with one peer id, so each
//! protocol becomes a field of [`Behaviour`] rather than a swarm of its own. [`EventLoop`] owns
//! the swarm and runs for as long as the monitor does; the rest of the monitor reaches it
//! through [`Network`], one command per network operation.

mod dht;
mod event_loop;
mod global_only;

use std::error::Error;

use anyhow::Context;
use libp2p::{
    PeerId, Swarm, SwarmBuilder, Transport,
    core::{muxing::StreamMuxerBox, transport::Boxed, upgrade::Version},
    dns, identify,
    identity::{Keypair, PublicKey},
    kad::{self, QueryResult, QueryStats, RecordKey, store::MemoryStore},
    noise,
    swarm::NetworkBehaviour,
    tcp, websocket, yamux,
};
use tokio::sync::mpsc;

pub use self::dht::{Bootnode, RecordLookup};
use self::event_loop::EventLoop;

const STOPPED: &str = "the network event loop stopped";

/// The same as Substrate nodes announce.
const PROTOCOL_VERSION: &str = "/substrate/1.0";

/// Lets node operators tell the monitor apart: a Substrate node logs the identify info of each
/// peer at the trace level of `sub-libp2p`.
const AGENT_VERSION: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

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
    identify: identify::Behaviour,
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
        .with_other_transport(transport)?
        .with_behaviour(|key| Behaviour {
            kademlia: dht::kademlia(key.public().to_peer_id(), genesis_hash, bootnodes),
            identify: identify(key.public()),
        })?
        .build())
}

fn identify(key: PublicKey) -> identify::Behaviour {
    let config = identify::Config::new(PROTOCOL_VERSION.to_string(), key)
        .with_agent_version(AGENT_VERSION.to_string());

    identify::Behaviour::new(config)
}

fn transport(
    key: &Keypair,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, Box<dyn Error + Send + Sync>> {
    // Peers announce their loopback and private addresses too, and a dial to one of those reaches
    // this machine or its network, so only global addresses are dialed. The filter sits right above
    // TCP and under DNS, so it checks the address that a name resolves to.
    let tcp = || global_only::Transport::new(tcp::tokio::Transport::new(tcp::Config::default()));

    // DNS sits above the handshake, as in libp2p's builder: when a resolved address takes the TCP
    // connection but fails the handshake, DNS goes on to the next one.
    let plain = dns::tokio::Transport::system(
        tcp()
            .upgrade(Version::V1Lazy)
            .authenticate(noise::Config::new(key)?)
            .multiplex(yamux::Config::default()),
    )?;

    // A node listens on WebSocket instead of plain TCP unless it is a validator or sets
    // --listen-addr, and a validator may set a WebSocket address there too. WebSocket needs DNS
    // under it rather than above, as TLS takes the host name of a /dns address.
    let websocket = websocket::Config::new(dns::tokio::Transport::system(tcp())?)
        .upgrade(Version::V1Lazy)
        .authenticate(noise::Config::new(key)?)
        .multiplex(yamux::Config::default());

    Ok(websocket
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .or_transport(plain.map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer))))
        .map(|either, _| either.into_inner())
        .boxed())
}

#[cfg(test)]
mod tests {
    use std::{io::ErrorKind, net::TcpListener};

    use libp2p::{Multiaddr, futures::StreamExt, swarm::SwarmEvent};

    use super::swarm;

    #[tokio::test]
    async fn service_on_loopback_gets_no_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();

        let mut swarm = swarm([0; 32], &[]).await.unwrap();
        for address in [
            format!("/ip4/127.0.0.1/tcp/{port}"),
            format!("/ip4/127.0.0.1/tcp/{port}/ws"),
            format!("/ip4/8.8.8.8/ip4/127.0.0.1/tcp/{port}"),
        ] {
            swarm.dial(address.parse::<Multiaddr>().unwrap()).unwrap();
        }

        let mut failed = 0;
        while failed < 3 {
            match swarm.select_next_some().await {
                SwarmEvent::OutgoingConnectionError { .. } => failed += 1,
                SwarmEvent::ConnectionEstablished { endpoint, .. } => {
                    panic!("connected to {}", endpoint.get_remote_address())
                }
                _ => {}
            }
        }

        let accepted = listener.accept().map(|(_, peer)| peer);
        assert!(
            accepted
                .as_ref()
                .is_err_and(|err| err.kind() == ErrorKind::WouldBlock),
            "{accepted:?}"
        );
    }
}

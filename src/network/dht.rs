//! The monitor's side of the chain's Kademlia DHT.

use std::str::FromStr;

use anyhow::{Context, bail};
use libp2p::{
    PeerId, StreamProtocol,
    kad::{
        self, GetRecordError, GetRecordOk, PeerRecord, QueryResult, RecordKey, store::MemoryStore,
    },
    multiaddr::{Multiaddr, Protocol},
};
use sha2::{Digest, Sha256};
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

#[derive(Default)]
pub struct RecordLookup {
    /// The distinct values that peers returned, in the order they came, neither decoded nor
    /// verified yet. Peers may hold different publications of the record.
    pub versions: Vec<Vec<u8>>,
    /// The lookup ran into Kademlia's query timeout before it reached the peers closest to the
    /// key. The versions found by then are kept.
    pub timed_out: bool,
    /// How many peers answered the lookup's requests, with the record or without it. Under 20,
    /// Kademlia's replication factor, the lookup cannot have heard from all the peers closest to
    /// the key.
    pub peers_answered: u32,
}

impl RecordLookup {
    /// Take in one event of the lookup, and return whether it was the last.
    fn update(&mut self, Progress { result, stats, last }: Progress) -> bool {
        let QueryResult::GetRecord(result) = result else {
            return last;
        };

        match result {
            Ok(GetRecordOk::FoundRecord(PeerRecord { record, .. })) => {
                if !self.versions.contains(&record.value) {
                    self.versions.push(record.value);
                }
            }
            Ok(GetRecordOk::FinishedWithNoAdditionalRecord { .. }) => {}
            Err(GetRecordError::Timeout { .. }) => self.timed_out = true,
            Err(GetRecordError::NotFound { .. }) => {}
        }

        if last {
            self.peers_answered = stats.num_successes();
        }

        last
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
        let Progress { stats, .. } = events.recv().await.context(STOPPED)?;
        if stats.num_successes() == 0 {
            bail!(
                "no boot node answered on this chain's Kademlia protocol: they are unreachable or \
                 belong to another chain"
            );
        }

        Ok(())
    }

    /// Look up the record of an authority discovery key, and return what the lookup found once
    /// it ends.
    pub async fn find_record(
        &self,
        authority_discovery_key: [u8; 32],
    ) -> anyhow::Result<RecordLookup> {
        let (progress, mut events) = mpsc::unbounded_channel();

        let cmd = Command::GetRecord {
            key: record_key(&authority_discovery_key),
            progress,
        };
        self.send(cmd).await?;

        let mut lookup = RecordLookup::default();
        while let Some(progress) = events.recv().await {
            if lookup.update(progress) {
                return Ok(lookup);
            }
        }
        bail!(STOPPED)
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

/// Substrate publishes the record of an authority discovery key under the key's SHA-256.
fn record_key(authority_discovery_key: &[u8; 32]) -> RecordKey {
    RecordKey::new(&Sha256::digest(authority_discovery_key))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use libp2p::kad::{
        GetRecordError, GetRecordOk, PeerRecord, QueryResult, QueryStats, Record, RecordKey,
    };

    use super::{Bootnode, Progress, RecordLookup};

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

    fn found(value: &[u8]) -> Progress {
        let record = Record::new(RecordKey::new(&[0]), value.to_vec());
        Progress {
            result: QueryResult::GetRecord(Ok(GetRecordOk::FoundRecord(PeerRecord {
                peer: None,
                record,
            }))),
            stats: QueryStats::empty(),
            last: false,
        }
    }

    fn finished() -> Progress {
        Progress {
            result: QueryResult::GetRecord(Ok(GetRecordOk::FinishedWithNoAdditionalRecord {
                cache_candidates: BTreeMap::new(),
            })),
            stats: QueryStats::empty(),
            last: true,
        }
    }

    fn timed_out() -> Progress {
        Progress {
            result: QueryResult::GetRecord(Err(GetRecordError::Timeout {
                key: RecordKey::new(&[0]),
            })),
            stats: QueryStats::empty(),
            last: true,
        }
    }

    #[test]
    fn lookup_keeps_each_version_once() {
        let mut lookup = RecordLookup::default();

        for progress in [found(b"old"), found(b"new"), found(b"old"), finished()] {
            lookup.update(progress);
        }

        assert_eq!(lookup.versions, [b"old".to_vec(), b"new".to_vec()]);
        assert!(!lookup.timed_out);
    }

    #[test]
    fn lookup_ends_on_last_event() {
        let mut lookup = RecordLookup::default();

        assert!(!lookup.update(found(b"record")));
        assert!(lookup.update(finished()));
    }

    #[test]
    fn timed_out_lookup_keeps_found_versions() {
        let mut lookup = RecordLookup::default();

        lookup.update(found(b"record"));
        lookup.update(timed_out());

        assert_eq!(lookup.versions, [b"record".to_vec()]);
        assert!(lookup.timed_out);
    }
}

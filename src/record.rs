//! Decoding and verification of a validator's authority discovery record, without the network.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};
use itertools::Itertools;
use libp2p::{Multiaddr, PeerId, identity::PublicKey};
use parity_scale_codec::Decode;
use prost::Message;

use crate::address;

/// The record format of authority discovery, written by hand from substrate's
/// [dht-v3.proto](https://github.com/paritytech/polkadot-sdk/blob/0f50c61a952e6ec776df557d8ffd54f341bd8b6a/substrate/client/authority-discovery/src/worker/schema/dht-v3.proto).
/// The field tags must match it: only they reach the wire.
mod schema {
    /// The serialized [`AuthorityRecord`] with the authority's and the peer's signatures over it;
    /// the peer's is missing before substrate#10317 (2021).
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SignedAuthorityRecord {
        #[prost(bytes = "vec", tag = "1")]
        pub record: Vec<u8>,
        #[prost(bytes = "vec", tag = "2")]
        pub auth_signature: Vec<u8>,
        #[prost(message, optional, tag = "3")]
        pub peer_signature: Option<PeerSignature>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PeerSignature {
        #[prost(bytes = "vec", tag = "1")]
        pub signature: Vec<u8>,
        #[prost(bytes = "vec", tag = "2")]
        pub public_key: Vec<u8>,
    }

    /// The signed body: the addresses, and, since RFC-0091, the time the authority built it.
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct AuthorityRecord {
        #[prost(bytes = "vec", repeated, tag = "1")]
        pub addresses: Vec<Vec<u8>>,
        #[prost(message, optional, tag = "2")]
        pub creation_time: Option<TimestampInfo>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TimestampInfo {
        /// Nanoseconds since the Unix epoch, SCALE-encoded as a `u128`.
        #[prost(bytes = "vec", tag = "1")]
        pub timestamp: Vec<u8>,
    }
}

/// The schnorrkel signing context of every sr25519 signature in Substrate, `SIGNING_CTX` in
/// sp-core.
const SIGNING_CONTEXT: &[u8] = b"substrate";

/// A record that passed both signature checks.
pub struct Record {
    /// The peer the addresses name, whose key signed the record.
    pub peer_id: PeerId,
    /// The published addresses, without those that name no peer.
    pub addresses: Vec<Multiaddr>,
    /// Absent from records older than RFC-0091.
    pub creation_time: Option<SystemTime>,
}

impl Record {
    /// Decode and verify one value found under the key `sha256(authority_discovery_key)`.
    ///
    /// Checks both signatures, as the node does, so a peer that answers the lookup cannot forge
    /// the record.
    pub fn decode(value: &[u8], authority_discovery_key: &[u8; 32]) -> anyhow::Result<Self> {
        let signed = schema::SignedAuthorityRecord::decode(value)
            .context("decode the signed authority record")?;
        signed.verify_authority_signature(authority_discovery_key)?;

        let record = schema::AuthorityRecord::decode(signed.record.as_slice())
            .context("decode the authority record")?;

        // Like the node, skip the addresses that name no peer.
        let addresses: Vec<_> = record
            .addresses
            .into_iter()
            .map(Multiaddr::try_from)
            .collect::<Result<Vec<_>, _>>()
            .context("decode a published address")?
            .into_iter()
            .filter(|published| address::peer_id(published).is_some())
            .collect();

        let peer_id = single_peer_id(&addresses)?;
        signed.verify_peer_signature(peer_id)?;

        let creation_time = record.creation_time.and_then(|time| {
            let nanos = u128::decode(&mut time.timestamp.as_slice()).ok()?;
            from_unix_nanos(nanos)
        });

        Ok(Self {
            peer_id,
            addresses,
            creation_time,
        })
    }
}

impl schema::SignedAuthorityRecord {
    fn verify_authority_signature(&self, authority_key: &[u8; 32]) -> anyhow::Result<()> {
        // schnorrkel's error implements `std::error::Error` only with its `failure` feature.
        let public = schnorrkel::PublicKey::from_bytes(authority_key)
            .map_err(anyhow::Error::msg)
            .context("decode the validator's key")?;
        let signature = schnorrkel::Signature::from_bytes(&self.auth_signature)
            .map_err(anyhow::Error::msg)
            .context("decode the authority signature")?;

        public
            .verify_simple(SIGNING_CONTEXT, &self.record, &signature)
            .map_err(anyhow::Error::msg)
            .context("the authority signature does not match the validator's key")
    }

    /// Also checks that the key is the peer the addresses name.
    fn verify_peer_signature(&self, peer_id: PeerId) -> anyhow::Result<()> {
        // A Polkadot node rejects a record without the peer signature too: its service turns on
        // `strict_record_validation`, which the library leaves off.
        let signature = self
            .peer_signature
            .as_ref()
            .context("the record carries no peer signature")?;

        let public_key = PublicKey::try_decode_protobuf(&signature.public_key)
            .context("decode the peer public key")?;

        ensure!(
            public_key.to_peer_id() == peer_id,
            "the peer public key does not match the peer the addresses name"
        );
        ensure!(
            public_key.verify(&self.record, &signature.signature),
            "the peer signature does not match its key"
        );
        Ok(())
    }
}

/// `Duration::from_nanos_u128` panics past `Duration::MAX`, and the nanoseconds come from a peer.
fn from_unix_nanos(nanos: u128) -> Option<SystemTime> {
    if nanos > Duration::MAX.as_nanos() {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_nanos_u128(nanos))
}

/// The authority publishes all its addresses for its own single peer.
fn single_peer_id(addresses: &[Multiaddr]) -> anyhow::Result<PeerId> {
    match addresses
        .iter()
        .filter_map(address::peer_id)
        .all_equal_value()
    {
        Ok(peer_id) => Ok(peer_id),
        Err(None) => bail!("the record carries no address that names a peer"),
        Err(Some(_)) => bail!("the published addresses name different peers"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use libp2p::{Multiaddr, identity::Keypair};
    use parity_scale_codec::Encode;
    use prost::Message as _;

    use super::{Record, SIGNING_CONTEXT, schema};

    fn keys() -> (schnorrkel::Keypair, [u8; 32], Keypair) {
        let authority = schnorrkel::Keypair::generate();
        let authority_key = authority.public.to_bytes();
        (authority, authority_key, Keypair::generate_ed25519())
    }

    fn address(peer: &Keypair, ip: &str) -> Multiaddr {
        format!("/ip4/{ip}/tcp/30333/p2p/{}", peer.public().to_peer_id())
            .parse()
            .unwrap()
    }

    fn encode_record(addresses: &[Multiaddr], creation_time: Option<u128>) -> Vec<u8> {
        schema::AuthorityRecord {
            addresses: addresses.iter().map(Multiaddr::to_vec).collect(),
            creation_time: creation_time
                .map(|time| schema::TimestampInfo { timestamp: time.encode() }),
        }
        .encode_to_vec()
    }

    fn sign_as_peer(peer: &Keypair, bytes: &[u8]) -> schema::PeerSignature {
        schema::PeerSignature {
            signature: peer.sign(bytes).unwrap(),
            public_key: peer.public().encode_protobuf(),
        }
    }

    fn sign_as_authority(
        authority: &schnorrkel::Keypair,
        record: Vec<u8>,
        peer_signature: Option<schema::PeerSignature>,
    ) -> Vec<u8> {
        schema::SignedAuthorityRecord {
            auth_signature: authority
                .sign_simple(SIGNING_CONTEXT, &record)
                .to_bytes()
                .to_vec(),
            peer_signature,
            record,
        }
        .encode_to_vec()
    }

    /// The value a node publishes: the record signed by the authority and by the peer.
    fn published(
        authority: &schnorrkel::Keypair,
        peer: &Keypair,
        addresses: &[Multiaddr],
        creation_time: Option<u128>,
    ) -> Vec<u8> {
        let record = encode_record(addresses, creation_time);
        let peer_signature = sign_as_peer(peer, &record);
        sign_as_authority(authority, record, Some(peer_signature))
    }

    /// Why the value is rejected, with the causes, so a test can tell which check failed.
    fn rejection(value: &[u8], authority_key: &[u8; 32]) -> String {
        match Record::decode(value, authority_key) {
            Ok(_) => panic!("the record was accepted"),
            Err(err) => format!("{err:#}"),
        }
    }

    #[test]
    fn verified_record_keeps_peer_addresses_and_time() {
        let (authority, authority_key, peer) = keys();
        let addresses = [address(&peer, "1.2.3.4"), address(&peer, "5.6.7.8")];
        let value = published(
            &authority,
            &peer,
            &addresses,
            Some(1_700_000_000_123_456_789),
        );

        let record = Record::decode(&value, &authority_key).unwrap();

        assert_eq!(record.peer_id, peer.public().to_peer_id());
        assert_eq!(record.addresses, addresses);
        assert_eq!(
            record.creation_time,
            Some(UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789))
        );
    }

    #[test]
    fn record_without_creation_time_has_none() {
        let (authority, authority_key, peer) = keys();
        let value = published(&authority, &peer, &[address(&peer, "1.2.3.4")], None);

        let record = Record::decode(&value, &authority_key).unwrap();

        assert_eq!(record.creation_time, None);
    }

    #[test]
    fn creation_time_beyond_system_time_is_none() {
        let (authority, authority_key, peer) = keys();
        let addresses = [address(&peer, "1.2.3.4")];
        let value = published(&authority, &peer, &addresses, Some(u128::MAX));

        let record = Record::decode(&value, &authority_key).unwrap();

        assert_eq!(record.creation_time, None);
    }

    #[test]
    fn address_that_names_no_peer_is_skipped() {
        let (authority, authority_key, peer) = keys();
        let named = address(&peer, "1.2.3.4");
        let unnamed: Multiaddr = "/ip4/5.6.7.8/tcp/30333".parse().unwrap();
        let value = published(&authority, &peer, &[named.clone(), unnamed], None);

        let record = Record::decode(&value, &authority_key).unwrap();

        assert_eq!(record.addresses, [named]);
    }

    #[test]
    fn record_signed_by_another_authority_is_rejected() {
        let (authority, _, peer) = keys();
        let other_key = schnorrkel::Keypair::generate().public.to_bytes();
        let value = published(&authority, &peer, &[address(&peer, "1.2.3.4")], None);

        assert!(rejection(&value, &other_key).contains("does not match the validator's key"));
    }

    #[test]
    fn record_without_peer_signature_is_rejected() {
        let (authority, authority_key, peer) = keys();
        let record = encode_record(&[address(&peer, "1.2.3.4")], None);
        let value = sign_as_authority(&authority, record, None);

        assert!(rejection(&value, &authority_key).contains("carries no peer signature"));
    }

    #[test]
    fn record_signed_by_another_peer_is_rejected() {
        let (authority, authority_key, peer) = keys();
        let record = encode_record(&[address(&peer, "1.2.3.4")], None);
        let impostor = sign_as_peer(&Keypair::generate_ed25519(), &record);
        let value = sign_as_authority(&authority, record, Some(impostor));

        assert!(
            rejection(&value, &authority_key)
                .contains("does not match the peer the addresses name")
        );
    }

    #[test]
    fn peer_signature_over_other_bytes_is_rejected() {
        let (authority, authority_key, peer) = keys();
        let record = encode_record(&[address(&peer, "1.2.3.4")], None);
        let value = sign_as_authority(&authority, record, Some(sign_as_peer(&peer, b"other")));

        assert!(
            rejection(&value, &authority_key).contains("peer signature does not match its key")
        );
    }

    #[test]
    fn record_whose_addresses_name_different_peers_is_rejected() {
        let (authority, authority_key, peer) = keys();
        let other = Keypair::generate_ed25519();
        let addresses = [address(&peer, "1.2.3.4"), address(&other, "5.6.7.8")];
        let value = published(&authority, &peer, &addresses, None);

        assert!(rejection(&value, &authority_key).contains("name different peers"));
    }

    #[test]
    fn record_without_addresses_is_rejected() {
        let (authority, authority_key, peer) = keys();
        let value = published(&authority, &peer, &[], None);

        assert!(rejection(&value, &authority_key).contains("no address that names a peer"));
    }
}

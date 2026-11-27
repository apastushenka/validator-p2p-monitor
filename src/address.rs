//! What kind of p2p address a peer publishes, told apart without the network.

use std::net::IpAddr;

use ip_network::IpNetwork;
use libp2p::{
    PeerId,
    multiaddr::{Multiaddr, Protocol},
};

/// Whether a dial to the address reaches a global IP and nothing else.
///
/// The address must hold exactly one IP. With two, it fails even if both are global: which of them
/// gets dialed depends on the transport, as libp2p-tcp takes the last one.
pub fn is_global(address: &Multiaddr) -> bool {
    let mut ips = address.iter().filter_map(|protocol| match protocol {
        Protocol::Ip4(ip) => Some(IpAddr::V4(ip)),
        Protocol::Ip6(ip) => Some(IpAddr::V6(ip)),
        _ => None,
    });

    // The canonical form turns an IPv4-mapped IPv6 address into its IPv4 one, which ip_network
    // would otherwise take for a global address.
    matches!(
        (ips.next(), ips.next()),
        (Some(ip), None) if IpNetwork::from(ip.to_canonical()).is_global()
    )
}

/// The peer id from the last `/p2p` of the address. An address through a relay has another
/// `/p2p` before `/p2p-circuit`, with the relay's peer id.
pub fn peer_id(address: &Multiaddr) -> Option<PeerId> {
    match address.iter().last() {
        Some(Protocol::P2p(peer_id)) => Some(peer_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use libp2p::PeerId;

    use super::{is_global, peer_id};

    fn check(address: &str) -> bool {
        is_global(&address.parse().unwrap())
    }

    #[test]
    fn global_address_is_dialed() {
        assert!(check("/ip4/8.8.8.8/tcp/30333"));
        assert!(check("/ip6/2001:4860:4860::8888/tcp/30333"));
        assert!(check("/ip6/::ffff:8.8.8.8/tcp/30333"));
    }

    #[test]
    fn local_and_private_addresses_are_not_dialed() {
        assert!(!check("/ip4/127.0.0.1/tcp/30333"));
        assert!(!check("/ip4/10.0.0.1/tcp/30333"));
        assert!(!check("/ip4/192.168.1.1/tcp/30333"));
        assert!(!check("/ip6/::1/tcp/30333"));
        assert!(!check("/ip6/::ffff:127.0.0.1/tcp/30333"));
        assert!(!check("/ip6/::ffff:10.0.0.1/tcp/30333"));
    }

    #[test]
    fn address_with_two_ips_is_not_dialed() {
        assert!(!check("/ip4/8.8.8.8/ip4/127.0.0.1/tcp/30333"));
        assert!(!check("/ip4/8.8.8.8/ip4/8.8.4.4/tcp/30333"));
    }

    #[test]
    fn address_without_ip_is_not_dialed() {
        assert!(!check("/dns/example.com/tcp/30333"));
    }

    #[test]
    fn relayed_address_names_its_destination() {
        let (relay, destination) = (PeerId::random(), PeerId::random());
        let address = format!("/ip4/8.8.8.8/tcp/30333/p2p/{relay}/p2p-circuit/p2p/{destination}");

        assert_eq!(peer_id(&address.parse().unwrap()), Some(destination));
    }

    #[test]
    fn address_without_p2p_names_no_peer() {
        assert_eq!(peer_id(&"/ip4/8.8.8.8/tcp/30333".parse().unwrap()), None);
    }
}

//! What kind of p2p address a peer publishes, told apart without the network.

use ip_network::IpNetwork;
use libp2p::multiaddr::{Multiaddr, Protocol};

/// Whether a dial to the address reaches a global IP and nothing else.
///
/// The address must hold exactly one IP. With two, it fails even if both are global: which of them
/// gets dialed depends on the transport, as libp2p-tcp takes the last one.
pub fn is_global(address: &Multiaddr) -> bool {
    let mut ips = address.iter().filter_map(|protocol| match protocol {
        Protocol::Ip4(ip) => Some(IpNetwork::from(ip)),
        Protocol::Ip6(ip) => Some(IpNetwork::from(ip)),
        _ => None,
    });

    matches!((ips.next(), ips.next()), (Some(ip), None) if ip.is_global())
}

#[cfg(test)]
mod tests {
    use super::is_global;

    fn check(address: &str) -> bool {
        is_global(&address.parse().unwrap())
    }

    #[test]
    fn global_address_is_dialed() {
        assert!(check("/ip4/8.8.8.8/tcp/30333"));
        assert!(check("/ip6/2001:4860:4860::8888/tcp/30333"));
    }

    #[test]
    fn local_and_private_addresses_are_not_dialed() {
        assert!(!check("/ip4/127.0.0.1/tcp/30333"));
        assert!(!check("/ip4/10.0.0.1/tcp/30333"));
        assert!(!check("/ip4/192.168.1.1/tcp/30333"));
        assert!(!check("/ip6/::1/tcp/30333"));
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
}

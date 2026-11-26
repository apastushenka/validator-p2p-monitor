//! A transport that dials global IP addresses only.
//!
//! libp2p has one of its own, but it does not check the IP that libp2p-tcp dials.

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use libp2p::{
    Multiaddr,
    core::transport::{DialOpts, ListenerId, TransportError, TransportEvent},
};

use crate::address::is_global;

pub struct Transport<T>(T);

impl<T> Transport<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }
}

impl<T: libp2p::Transport + Unpin> libp2p::Transport for Transport<T> {
    type Output = T::Output;
    type Error = T::Error;
    type ListenerUpgrade = T::ListenerUpgrade;
    type Dial = T::Dial;

    fn listen_on(
        &mut self,
        id: ListenerId,
        addr: Multiaddr,
    ) -> Result<(), TransportError<Self::Error>> {
        self.0.listen_on(id, addr)
    }

    fn remove_listener(&mut self, id: ListenerId) -> bool {
        self.0.remove_listener(id)
    }

    fn dial(
        &mut self,
        addr: Multiaddr,
        opts: DialOpts,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        if !is_global(&addr) {
            return Err(TransportError::MultiaddrNotSupported(addr));
        }
        self.0.dial(addr, opts)
    }

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

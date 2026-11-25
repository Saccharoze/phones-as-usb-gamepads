//! Who sent a frame: a UDP address, or one browser's WebSocket connection.

use std::fmt;
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Peer {
    Udp(SocketAddr),
    /// `id` tells apart two connections from the same address.
    Web { id: u64, addr: SocketAddr },
}

impl Peer {
    pub fn addr(&self) -> SocketAddr {
        match *self {
            Peer::Udp(addr) | Peer::Web { addr, .. } => addr,
        }
    }

    /// The `via` field of STATUS.
    pub fn via(&self) -> &'static str {
        match self {
            Peer::Udp(_) => "udp",
            Peer::Web { .. } => "web",
        }
    }
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Peer::Udp(addr) => write!(f, "{addr}"),
            Peer::Web { id, addr } => write!(f, "{addr} (web #{id})"),
        }
    }
}

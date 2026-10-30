//! The transport seam. Everything above it (framing, reliability, server, client) is backend-agnostic.

use std::time::Duration;

/// Identifies one remote peer as seen by the server. Client ids must be non-zero, because
/// the client side uses [`SERVER_PEER`] for the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u32);

/// The id the client side uses for its single connection to the server.
pub const SERVER_PEER: ClientId = ClientId(0);

/// Delivery class requested by the caller. The networking layer sends every datagram on the
/// transport's unreliable path and provides `Reliable` and `Snapshots` itself (see ADR 0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Acknowledged, delivered once and in order.
    Reliable,
    /// Best effort: may be lost or reordered, never delivered twice.
    Unreliable,
    /// Best effort, newest wins: a snapshot older than the last accepted one is dropped.
    Snapshots,
}

/// Why a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    /// The server already has the maximum number of players.
    Full,
    /// No packet was received within the timeout.
    Timeout,
    /// Closed locally.
    Closed,
}

/// Connection lifecycle reported by a transport or by the networking handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetEvent {
    /// A peer connected.
    Connected(ClientId),
    /// A peer disconnected.
    Disconnected(ClientId, DisconnectReason),
}

/// A datagram transport. The server endpoint addresses peers by [`ClientId`]; the client
/// endpoint has one peer, [`SERVER_PEER`], and ignores the `client` argument.
pub trait Transport {
    /// Advances the transport by `dt`. Called once per frame by the server and client handles.
    fn poll(&mut self, dt: Duration);
    /// Sends one datagram to `client` (server) or to the server (client).
    fn send(&mut self, client: ClientId, channel: Channel, data: &[u8]);
    /// Sends one datagram to every connected peer.
    fn broadcast(&mut self, channel: Channel, data: &[u8]);
    /// Takes the next received datagram from `client`, if any.
    fn recv(&mut self, client: ClientId, channel: Channel) -> Option<Vec<u8>>;
    /// Takes the connection events that happened since the previous call.
    fn events(&mut self) -> Vec<NetEvent>;
    /// Round-trip time to `client`, if connected.
    fn rtt(&self, client: ClientId) -> Option<Duration>;
    /// Closes the connection to `client` and tells the peer why.
    fn disconnect(&mut self, client: ClientId, reason: DisconnectReason);
}

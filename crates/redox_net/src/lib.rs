//! Networking foundation for the RedOx Engine: transport trait, authoritative server and client,
//! reliable, unreliable and snapshot channels, message codec and a deterministic loopback wire.
//!
//! Reliability and ordering are implemented in this crate on top of the transport's datagrams, so
//! every backend behaves the same. Decision record: `docs/adr/0008-networking.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod error;
mod frame;
mod loopback;
mod message;
mod peer;
mod reliable;
mod rng;
mod sequence;
mod server;
mod tick;
mod transport;

pub use client::{ConnectionState, NetClient};
pub use error::NetError;
pub use frame::{
    MAX_PACKET_BYTES, MAX_RELIABLE_PAYLOAD, MAX_SNAPSHOT_PAYLOAD, MAX_UNRELIABLE_PAYLOAD,
};
pub use loopback::{LoopbackConditions, LoopbackTransport, LoopbackWire};
pub use message::{Message, decode, encode, message_id};
pub use rng::SplitMix64;
pub use server::{CLIENT_TIMEOUT, MAX_PLAYERS, NetServer, RESEND_INTERVAL};
pub use tick::{
    MAX_TICKS_PER_POLL, SERVER_TICK_DURATION, SERVER_TICK_RATE_HZ, TickEstimator, drain_fixed_ticks,
};
pub use transport::{Channel, ClientId, DisconnectReason, NetEvent, SERVER_PEER, Transport};

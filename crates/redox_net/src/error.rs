//! Error type shared by framing, codec and the server and client handles.

use crate::transport::ClientId;

/// Errors reported by `redox_net`. Malformed input from the wire is never a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// The payload does not fit the limit of its channel.
    #[error("payload of {len} bytes exceeds the {max}-byte limit for this channel")]
    TooLarge {
        /// Size of the rejected payload.
        len: usize,
        /// Largest accepted size.
        max: usize,
    },
    /// A datagram or message body could not be parsed.
    #[error("malformed datagram: {0}")]
    Malformed(&'static str),
    /// A decoded message carries another id than the requested type.
    #[error("message id mismatch: expected {expected}, got {got}")]
    WrongMessageId {
        /// Id of the requested type.
        expected: u16,
        /// Id found in the bytes.
        got: u16,
    },
    /// postcard failed to encode or decode a message body.
    #[error("message codec error: {0}")]
    Codec(String),
    /// The server has no connection for this client.
    #[error("client {0:?} is not connected")]
    UnknownClient(ClientId),
    /// The client connection is closed.
    #[error("the connection is closed")]
    NotConnected,
}

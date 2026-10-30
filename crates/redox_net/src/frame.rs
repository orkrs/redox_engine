//! Wire framing. Every datagram starts with a kind byte and a `u32` sequence number.

use crate::error::NetError;
use crate::rng::SplitMix64;

/// Largest datagram produced by the layer, below a typical 1500-byte Ethernet MTU.
pub const MAX_PACKET_BYTES: usize = 1200;

const KIND_UNRELIABLE: u8 = 0;
const KIND_RELIABLE: u8 = 1;
const KIND_ACK: u8 = 2;
const KIND_SNAPSHOT: u8 = 3;

const UNRELIABLE_HEADER: usize = 1 + 4;
const RELIABLE_HEADER: usize = 1 + 4;
const SNAPSHOT_HEADER: usize = 1 + 4 + 8;

/// Largest payload accepted on the unreliable channel. Fragmentation is not implemented.
pub const MAX_UNRELIABLE_PAYLOAD: usize = MAX_PACKET_BYTES - UNRELIABLE_HEADER;
/// Largest payload accepted on the reliable channel. Fragmentation is not implemented.
pub const MAX_RELIABLE_PAYLOAD: usize = MAX_PACKET_BYTES - RELIABLE_HEADER;
/// Largest payload accepted on the snapshot channel.
pub const MAX_SNAPSHOT_PAYLOAD: usize = MAX_PACKET_BYTES - SNAPSHOT_HEADER;

/// One datagram on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Best-effort payload with a sequence number for duplicate suppression.
    Unreliable {
        /// Sender-side sequence number.
        seq: u32,
        /// Application bytes.
        payload: Vec<u8>,
    },
    /// Payload that the receiver must acknowledge.
    Reliable {
        /// Sender-side sequence number.
        seq: u32,
        /// Application bytes.
        payload: Vec<u8>,
    },
    /// Cumulative acknowledgement: every reliable frame below `next_expected` arrived.
    Ack {
        /// First reliable sequence number not yet received.
        next_expected: u32,
    },
    /// Snapshot with its server tick.
    Snapshot {
        /// Sender-side snapshot sequence number.
        seq: u32,
        /// Server tick at which the snapshot was taken.
        tick: u64,
        /// Application bytes.
        payload: Vec<u8>,
    },
}

impl Frame {
    /// Serializes the frame. Fails when the result would exceed [`MAX_PACKET_BYTES`].
    pub fn encode(&self) -> Result<Vec<u8>, NetError> {
        let mut out = Vec::with_capacity(MAX_PACKET_BYTES.min(SNAPSHOT_HEADER + 64));
        match self {
            Frame::Unreliable { seq, payload } => {
                push_header(&mut out, KIND_UNRELIABLE, *seq);
                out.extend_from_slice(payload);
            }
            Frame::Reliable { seq, payload } => {
                push_header(&mut out, KIND_RELIABLE, *seq);
                out.extend_from_slice(payload);
            }
            Frame::Ack { next_expected } => push_header(&mut out, KIND_ACK, *next_expected),
            Frame::Snapshot { seq, tick, payload } => {
                push_header(&mut out, KIND_SNAPSHOT, *seq);
                out.extend_from_slice(&tick.to_le_bytes());
                out.extend_from_slice(payload);
            }
        }
        if out.len() > MAX_PACKET_BYTES {
            return Err(NetError::TooLarge {
                len: out.len(),
                max: MAX_PACKET_BYTES,
            });
        }
        Ok(out)
    }

    /// Parses one datagram. Any byte sequence yields `Ok` or `Err`, never a panic.
    pub fn decode(bytes: &[u8]) -> Result<Frame, NetError> {
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(NetError::Malformed("datagram larger than the packet limit"));
        }
        let (&kind, rest) = bytes
            .split_first()
            .ok_or(NetError::Malformed("empty datagram"))?;
        let seq = read_u32(rest)?;
        let body = rest.get(4..).unwrap_or(&[]);
        match kind {
            KIND_UNRELIABLE => Ok(Frame::Unreliable {
                seq,
                payload: body.to_vec(),
            }),
            KIND_RELIABLE => Ok(Frame::Reliable {
                seq,
                payload: body.to_vec(),
            }),
            KIND_ACK if body.is_empty() => Ok(Frame::Ack {
                next_expected: seq,
            }),
            KIND_ACK => Err(NetError::Malformed("acknowledgement with a payload")),
            KIND_SNAPSHOT => {
                let tick = read_u64(body)?;
                let payload = body.get(8..).unwrap_or(&[]);
                Ok(Frame::Snapshot {
                    seq,
                    tick,
                    payload: payload.to_vec(),
                })
            }
            _ => Err(NetError::Malformed("unknown frame kind")),
        }
    }
}

fn push_header(out: &mut Vec<u8>, kind: u8, seq: u32) {
    out.push(kind);
    out.extend_from_slice(&seq.to_le_bytes());
}

fn read_u32(bytes: &[u8]) -> Result<u32, NetError> {
    let raw: [u8; 4] = bytes
        .get(..4)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(NetError::Malformed("truncated sequence number"))?;
    Ok(u32::from_le_bytes(raw))
}

fn read_u64(bytes: &[u8]) -> Result<u64, NetError> {
    let raw: [u8; 8] = bytes
        .get(..8)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(NetError::Malformed("truncated snapshot tick"))?;
    Ok(u64::from_le_bytes(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_kind_roundtrips() {
        let frames = [
            Frame::Unreliable {
                seq: 3,
                payload: vec![1, 2],
            },
            Frame::Reliable {
                seq: u32::MAX,
                payload: vec![9; 10],
            },
            Frame::Ack { next_expected: 9 },
            Frame::Snapshot {
                seq: 4,
                tick: 1 << 40,
                payload: vec![5],
            },
        ];
        for frame in frames {
            let bytes = frame.encode().expect("small frames encode");
            assert_eq!(Frame::decode(&bytes), Ok(frame));
        }
    }

    #[test]
    fn an_encoding_over_the_limit_is_refused() {
        let frame = Frame::Reliable {
            seq: 0,
            payload: vec![0; MAX_RELIABLE_PAYLOAD + 1],
        };
        assert!(matches!(frame.encode(), Err(NetError::TooLarge { .. })));
    }

    #[test]
    fn an_oversized_datagram_is_rejected() {
        let bytes = vec![0u8; MAX_PACKET_BYTES + 1];
        assert!(matches!(Frame::decode(&bytes), Err(NetError::Malformed(_))));
    }

    #[test]
    fn garbled_bytes_never_panic() {
        let mut rng = SplitMix64::new(0xFEED);
        for _ in 0..1000 {
            let len = (rng.next_u64() % (MAX_PACKET_BYTES as u64 + 16)) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
            let _ = Frame::decode(&bytes);
        }
    }
}

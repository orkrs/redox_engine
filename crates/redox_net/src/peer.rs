//! Per-connection state shared by the server (one per client) and the client (one for the server).

use std::collections::VecDeque;
use std::time::Duration;

use crate::error::NetError;
use crate::frame::{Frame, MAX_RELIABLE_PAYLOAD, MAX_SNAPSHOT_PAYLOAD, MAX_UNRELIABLE_PAYLOAD};
use crate::reliable::{ReliableReceiver, ReliableSender};
use crate::sequence::{DuplicateFilter, LatestGate};
use crate::transport::Channel;

/// Unreliable packets beyond this backlog are dropped, oldest first.
const UNRELIABLE_INBOX_CAPACITY: usize = 1024;

#[derive(Default)]
struct Inbox {
    unreliable: VecDeque<Vec<u8>>,
    reliable: VecDeque<Vec<u8>>,
    snapshot: Option<Vec<u8>>,
}

/// Connection state for one remote peer.
pub(crate) struct Peer {
    reliable_tx: ReliableSender,
    reliable_rx: ReliableReceiver,
    unreliable_seq: u32,
    unreliable_filter: DuplicateFilter,
    snapshot_seq: u32,
    snapshot_gate: LatestGate,
    /// Encoded unreliable and snapshot frames waiting for the next flush.
    outgoing: Vec<Vec<u8>>,
    inbox: Inbox,
    last_heard: Duration,
}

impl Peer {
    /// A peer that was last heard from at `now`.
    pub(crate) fn new(now: Duration) -> Self {
        Self {
            reliable_tx: ReliableSender::default(),
            reliable_rx: ReliableReceiver::default(),
            unreliable_seq: 0,
            unreliable_filter: DuplicateFilter::default(),
            snapshot_seq: 0,
            snapshot_gate: LatestGate::default(),
            outgoing: Vec::new(),
            inbox: Inbox::default(),
            last_heard: now,
        }
    }

    /// Queues an application payload on `channel`. `tick` is stamped on snapshots.
    pub(crate) fn send(
        &mut self,
        channel: Channel,
        data: &[u8],
        tick: u64,
    ) -> Result<(), NetError> {
        match channel {
            Channel::Reliable => {
                check_len(data.len(), MAX_RELIABLE_PAYLOAD)?;
                self.reliable_tx.queue(data.to_vec());
            }
            Channel::Unreliable => {
                check_len(data.len(), MAX_UNRELIABLE_PAYLOAD)?;
                let seq = self.unreliable_seq;
                self.unreliable_seq = self.unreliable_seq.wrapping_add(1);
                let frame = Frame::Unreliable {
                    seq,
                    payload: data.to_vec(),
                };
                self.outgoing.push(frame.encode()?);
            }
            Channel::Snapshots => {
                check_len(data.len(), MAX_SNAPSHOT_PAYLOAD)?;
                let seq = self.snapshot_seq;
                self.snapshot_seq = self.snapshot_seq.wrapping_add(1);
                let frame = Frame::Snapshot {
                    seq,
                    tick,
                    payload: data.to_vec(),
                };
                self.outgoing.push(frame.encode()?);
            }
        }
        Ok(())
    }

    /// Encoded datagrams to transmit now: fresh and due reliable frames, queued unreliable frames,
    /// and one acknowledgement.
    pub(crate) fn flush(&mut self, now: Duration, resend_after: Duration) -> Vec<Vec<u8>> {
        let mut out = std::mem::take(&mut self.outgoing);
        for frame in self.reliable_tx.due(now, resend_after) {
            push_encoded(&mut out, &frame);
        }
        let ack = Frame::Ack {
            next_expected: self.reliable_rx.next_expected(),
        };
        push_encoded(&mut out, &ack);
        out
    }

    /// Applies one received frame. Returns the server tick when a fresh snapshot was accepted.
    pub(crate) fn receive(&mut self, frame: Frame, now: Duration) -> Option<u64> {
        self.last_heard = now;
        match frame {
            Frame::Unreliable { seq, payload } => {
                if self.unreliable_filter.accept(seq) {
                    push_bounded(&mut self.inbox.unreliable, payload);
                }
                None
            }
            Frame::Reliable { seq, payload } => {
                self.reliable_rx.on_data(seq, payload);
                while let Some(ready) = self.reliable_rx.pop() {
                    self.inbox.reliable.push_back(ready);
                }
                None
            }
            Frame::Ack { next_expected } => {
                self.reliable_tx.on_ack(next_expected);
                None
            }
            Frame::Snapshot { seq, tick, payload } => {
                if self.snapshot_gate.accept(seq) {
                    self.inbox.snapshot = Some(payload);
                    Some(tick)
                } else {
                    None
                }
            }
        }
    }

    /// Takes the next payload received on `channel`.
    pub(crate) fn pop(&mut self, channel: Channel) -> Option<Vec<u8>> {
        match channel {
            Channel::Reliable => self.inbox.reliable.pop_front(),
            Channel::Unreliable => self.inbox.unreliable.pop_front(),
            Channel::Snapshots => self.inbox.snapshot.take(),
        }
    }

    /// True when nothing was received for longer than `timeout`.
    pub(crate) fn is_silent(&self, now: Duration, timeout: Duration) -> bool {
        now.saturating_sub(self.last_heard) > timeout
    }
}

fn check_len(len: usize, max: usize) -> Result<(), NetError> {
    if len > max {
        Err(NetError::TooLarge { len, max })
    } else {
        Ok(())
    }
}

fn push_bounded(queue: &mut VecDeque<Vec<u8>>, payload: Vec<u8>) {
    if queue.len() >= UNRELIABLE_INBOX_CAPACITY {
        queue.pop_front();
    }
    queue.push_back(payload);
}

fn push_encoded(out: &mut Vec<Vec<u8>>, frame: &Frame) {
    match frame.encode() {
        Ok(bytes) => out.push(bytes),
        Err(error) => log::warn!("dropping outgoing frame: {error}"),
    }
}

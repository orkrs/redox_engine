//! Acknowledged, ordered delivery over datagrams that may be lost, duplicated or reordered.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use crate::frame::Frame;
use crate::sequence::seq_after;

/// How far ahead of the next expected sequence number the receiver buffers packets.
pub const RELIABLE_WINDOW: u32 = 1024;

struct Pending {
    seq: u32,
    payload: Vec<u8>,
    last_sent: Option<Duration>,
}

/// Sender side: queues payloads and retransmits them until they are acknowledged.
#[derive(Default)]
pub struct ReliableSender {
    next_seq: u32,
    pending: VecDeque<Pending>,
}

impl ReliableSender {
    /// Queues a payload. It is sent on the next [`ReliableSender::due`] call.
    pub fn queue(&mut self, payload: Vec<u8>) {
        self.pending.push_back(Pending {
            seq: self.next_seq,
            payload,
            last_sent: None,
        });
        self.next_seq = self.next_seq.wrapping_add(1);
    }

    /// Forgets every packet below `next_expected`, which the receiver has confirmed.
    pub fn on_ack(&mut self, next_expected: u32) {
        self.pending
            .retain(|pending| !seq_after(next_expected, pending.seq));
    }

    /// Frames to transmit at `now`: packets never sent, and packets not sent for `resend_after`.
    pub fn due(&mut self, now: Duration, resend_after: Duration) -> Vec<Frame> {
        let mut frames = Vec::new();
        for pending in &mut self.pending {
            let is_due = pending
                .last_sent
                .is_none_or(|sent| now.saturating_sub(sent) >= resend_after);
            if is_due {
                pending.last_sent = Some(now);
                frames.push(Frame::Reliable {
                    seq: pending.seq,
                    payload: pending.payload.clone(),
                });
            }
        }
        frames
    }
}

/// Receiver side: delivers payloads exactly once and in sequence order.
#[derive(Default)]
pub struct ReliableReceiver {
    next_expected: u32,
    buffered: BTreeMap<u32, Vec<u8>>,
    ready: VecDeque<Vec<u8>>,
}

impl ReliableReceiver {
    /// First sequence number not yet delivered. Sent back to the sender in acknowledgements.
    pub fn next_expected(&self) -> u32 {
        self.next_expected
    }

    /// Handles one reliable frame. Duplicates and packets beyond the window are ignored.
    pub fn on_data(&mut self, seq: u32, payload: Vec<u8>) {
        let ahead = seq.wrapping_sub(self.next_expected);
        if ahead >= RELIABLE_WINDOW {
            return;
        }
        if ahead == 0 {
            self.ready.push_back(payload);
            self.next_expected = self.next_expected.wrapping_add(1);
            self.release_buffered();
        } else {
            self.buffered.entry(seq).or_insert(payload);
        }
    }

    /// Takes the next payload that is ready to be delivered in order.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        self.ready.pop_front()
    }

    fn release_buffered(&mut self) {
        while let Some(payload) = self.buffered.remove(&self.next_expected) {
            self.ready.push_back(payload);
            self.next_expected = self.next_expected.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_receiver_reorders_and_drops_duplicates() {
        let mut receiver = ReliableReceiver::default();
        receiver.on_data(1, vec![1]);
        assert_eq!(receiver.pop(), None);
        receiver.on_data(0, vec![0]);
        receiver.on_data(0, vec![0]);
        assert_eq!(receiver.pop(), Some(vec![0]));
        assert_eq!(receiver.pop(), Some(vec![1]));
        assert_eq!(receiver.pop(), None);
        assert_eq!(receiver.next_expected(), 2);
    }

    #[test]
    fn an_acknowledgement_releases_confirmed_packets() {
        let mut sender = ReliableSender::default();
        sender.queue(vec![0]);
        sender.queue(vec![1]);
        sender.queue(vec![2]);
        let first = sender.due(Duration::ZERO, Duration::from_millis(100));
        assert_eq!(first.len(), 3);
        sender.on_ack(2);
        let after = sender.due(Duration::from_secs(1), Duration::from_millis(100));
        assert_eq!(after.len(), 1, "only sequence 2 is still unacknowledged");
    }
}

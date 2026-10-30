//! Sequence numbers with wrap-around, and the two receive-side filters built on them.

/// Packets further behind the newest sequence than this are rejected by [`DuplicateFilter`].
const DUPLICATE_WINDOW: u32 = 64;

/// True when `a` comes after `b` in wrap-around order.
pub fn seq_after(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

/// Accepts only sequence numbers newer than the last accepted one. Used for snapshots, where an
/// older snapshot is useless once a newer one has arrived.
#[derive(Debug, Default, Clone, Copy)]
pub struct LatestGate {
    latest: Option<u32>,
}

impl LatestGate {
    /// Returns `true` and records `seq` when it is newer than everything seen so far.
    pub fn accept(&mut self, seq: u32) -> bool {
        match self.latest {
            Some(latest) if !seq_after(seq, latest) => false,
            _ => {
                self.latest = Some(seq);
                true
            }
        }
    }
}

/// Rejects duplicates and anything older than the window. Used for unreliable packets, which may
/// arrive out of order but must never reach the game twice.
#[derive(Debug, Default, Clone, Copy)]
pub struct DuplicateFilter {
    newest: Option<u32>,
    /// Bit `i` is set when `newest - i` was seen.
    seen: u64,
}

impl DuplicateFilter {
    /// Returns `true` the first time a sequence number is offered, while it is inside the window.
    pub fn accept(&mut self, seq: u32) -> bool {
        let Some(newest) = self.newest else {
            self.newest = Some(seq);
            self.seen = 1;
            return true;
        };
        if seq_after(seq, newest) {
            let shift = seq.wrapping_sub(newest);
            self.seen = if shift >= DUPLICATE_WINDOW {
                0
            } else {
                self.seen << shift
            };
            self.seen |= 1;
            self.newest = Some(seq);
            return true;
        }
        let age = newest.wrapping_sub(seq);
        if age >= DUPLICATE_WINDOW {
            return false;
        }
        let bit = 1u64 << age;
        if self.seen & bit != 0 {
            return false;
        }
        self.seen |= bit;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_gate_drops_older_and_repeated_snapshots() {
        let mut gate = LatestGate::default();
        assert!(gate.accept(6));
        assert!(!gate.accept(5), "snapshot 5 arrives after 6 and is ignored");
        assert!(!gate.accept(6));
        assert!(gate.accept(7));
    }

    #[test]
    fn the_duplicate_filter_accepts_reordered_packets_once() {
        let mut filter = DuplicateFilter::default();
        assert!(filter.accept(5));
        assert!(!filter.accept(5));
        assert!(filter.accept(3), "a late packet inside the window is new");
        assert!(!filter.accept(3));
    }

    #[test]
    fn the_duplicate_filter_rejects_packets_outside_the_window() {
        let mut filter = DuplicateFilter::default();
        assert!(filter.accept(100));
        assert!(!filter.accept(30));
    }
}

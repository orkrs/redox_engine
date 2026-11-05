//! Replaceable time for the scenario tests (EDT-A7): the editor never reads the wall clock here.

use std::sync::Arc;

use parking_lot::Mutex;
use redox_editor_core::autosave::Clock;

/// A clock that moves only when a test advances it. Clones share the same time.
#[derive(Clone, Debug, Default)]
pub struct TestClock {
    now: Arc<Mutex<f64>>,
}

impl TestClock {
    /// Creates a clock at time zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the time forward by `secs` seconds.
    pub fn advance(&self, secs: u64) {
        *self.now.lock() += secs as f64;
    }

    /// The current time in seconds.
    pub fn now_secs(&self) -> f64 {
        *self.now.lock()
    }
}

impl Clock for TestClock {
    fn now_secs(&self) -> f64 {
        TestClock::now_secs(self)
    }
}

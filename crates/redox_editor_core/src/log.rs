//! Console log shared by the editor front ends.

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

/// Number of console lines kept; the oldest lines are dropped first.
pub const LOG_CAPACITY: usize = 200;

/// Thread-safe ring buffer of console lines. Clones share the same buffer.
#[derive(Clone, Debug, Default)]
pub struct EditorLog {
    lines: Arc<Mutex<VecDeque<String>>>,
}

impl EditorLog {
    /// Creates an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `line` exactly as given; the oldest line is dropped when the buffer is full.
    pub fn push(&self, line: impl Into<String>) {
        let mut lines = self.lines.lock();
        lines.push_back(line.into());
        while lines.len() > LOG_CAPACITY {
            lines.pop_front();
        }
    }

    /// Appends an `[INFO]` line.
    pub fn info(&self, message: impl AsRef<str>) {
        self.push(format!("[INFO] {}", message.as_ref()));
    }

    /// Appends a `[WARN]` line.
    pub fn warn(&self, message: impl AsRef<str>) {
        self.push(format!("[WARN] {}", message.as_ref()));
    }

    /// Appends an `[ERROR]` line.
    pub fn error(&self, message: impl AsRef<str>) {
        self.push(format!("[ERROR] {}", message.as_ref()));
    }

    /// Copies all lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().iter().cloned().collect()
    }

    /// Removes all lines.
    pub fn clear(&self) {
        self.lines.lock().clear();
    }
}

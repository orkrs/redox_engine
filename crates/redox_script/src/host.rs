//! Backend independent host state: the game clock and the bounded log buffer.

use std::collections::VecDeque;

/// Longest log message kept, in bytes; longer messages are truncated.
pub(crate) const MAX_LOG_BYTES: usize = 4096;
/// Most log messages buffered per instance; the oldest are dropped first.
pub(crate) const MAX_BUFFERED_LOGS: usize = 1000;

/// Per-instance state shared between the host functions and the runtime.
#[derive(Debug, Default)]
pub(crate) struct HostData {
    /// Accumulated game time in seconds.
    pub(crate) clock: f32,
    logs: VecDeque<String>,
}

impl HostData {
    /// Buffers a log message from raw guest bytes.
    ///
    /// The message is truncated to [`MAX_LOG_BYTES`] and invalid UTF-8 is replaced lossily.
    pub(crate) fn push_log(&mut self, bytes: &[u8]) {
        let capped = bytes.get(..MAX_LOG_BYTES).unwrap_or(bytes);
        if self.logs.len() >= MAX_BUFFERED_LOGS {
            self.logs.pop_front();
        }
        self.logs
            .push_back(String::from_utf8_lossy(capped).into_owned());
    }

    /// Removes and returns all buffered messages, oldest first.
    pub(crate) fn take_logs(&mut self) -> Vec<String> {
        self.logs.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_are_truncated() {
        let mut host = HostData::default();
        host.push_log(&vec![b'a'; MAX_LOG_BYTES * 2]);
        assert_eq!(host.take_logs()[0].len(), MAX_LOG_BYTES);
    }

    #[test]
    fn invalid_utf8_is_logged_lossily() {
        let mut host = HostData::default();
        host.push_log(&[b'o', b'k', 0xFF]);
        assert_eq!(host.take_logs(), vec!["ok\u{FFFD}"]);
    }

    #[test]
    fn oldest_messages_are_dropped_when_full() {
        let mut host = HostData::default();
        for i in 0..MAX_BUFFERED_LOGS + 5 {
            host.push_log(i.to_string().as_bytes());
        }
        let logs = host.take_logs();
        assert_eq!(logs.len(), MAX_BUFFERED_LOGS);
        assert_eq!(logs[0], "5");
    }
}

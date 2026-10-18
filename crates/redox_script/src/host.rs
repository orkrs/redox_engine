//! Backend independent host state: the game clock and the bounded log buffer.

use std::collections::{HashMap, VecDeque};

use crate::{ScriptEvent, WorldQueries};

/// Longest log message kept, in bytes; longer messages are truncated.
pub(crate) const MAX_LOG_BYTES: usize = 4096;
/// Most log messages buffered per instance; the oldest are dropped first.
pub(crate) const MAX_BUFFERED_LOGS: usize = 1000;

/// Longest accepted name for `get_f32` / `set_f32` / `emit_event`, in bytes.
pub(crate) const MAX_NAME_BYTES: usize = 64;
/// Most distinct outputs stored per instance; new names beyond it are dropped.
pub(crate) const MAX_OUTPUTS: usize = 256;
/// Most events queued per instance between two `take_events` calls.
pub(crate) const MAX_EVENTS: usize = 256;

/// Most `raycast` queries answered per call into the script.
pub(crate) const MAX_RAYS_PER_CALL: u32 = 64;
/// Longest ray the host is asked to cast, in metres.
pub(crate) const MAX_RAY_DISTANCE: f32 = 1000.0;

/// The installed world, if any.
#[derive(Default)]
struct WorldSlot(Option<Box<dyn WorldQueries>>);

impl std::fmt::Debug for WorldSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "WorldSlot(set)"
        } else {
            "WorldSlot(none)"
        })
    }
}

/// Reads a script supplied name `(ptr, len)` out of guest memory.
///
/// Returns `None` for negative values, ranges outside `memory`, names longer than
/// [`MAX_NAME_BYTES`] and invalid UTF-8.
pub(crate) fn read_name(memory: &[u8], ptr: i32, len: i32) -> Option<&str> {
    let start = usize::try_from(ptr).ok()?;
    let len = usize::try_from(len).ok().filter(|l| *l <= MAX_NAME_BYTES)?;
    let bytes = memory.get(start..start.checked_add(len)?)?;
    std::str::from_utf8(bytes).ok()
}

/// Per-instance state shared between the host functions and the runtime.
#[derive(Debug, Default)]
pub(crate) struct HostData {
    /// Accumulated game time in seconds.
    pub(crate) clock: f32,
    logs: VecDeque<String>,
    inputs: HashMap<String, f32>,
    outputs: HashMap<String, f32>,
    events: Vec<ScriptEvent>,
    world: WorldSlot,
    rays_left: u32,
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

    /// Installs the world answering `raycast`.
    pub(crate) fn set_world(&mut self, world: Box<dyn WorldQueries>) {
        self.world = WorldSlot(Some(world));
    }

    /// Starts a call into the script: the ray allowance is renewed.
    pub(crate) fn begin_call(&mut self) {
        self.rays_left = MAX_RAYS_PER_CALL;
    }

    /// Answers `env.raycast`: the hit distance or -1.0 (no world, bad arguments, allowance used up,
    /// or no hit). The direction is normalised and the distance capped before the host is asked.
    pub(crate) fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max: f32) -> f32 {
        const MISS: f32 = -1.0;
        let finite = origin.iter().chain(&direction).all(|v| v.is_finite());
        let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
        if !finite || !max.is_finite() || max <= 0.0 || length <= 0.0 {
            return MISS;
        }
        let Some(world) = self.world.0.as_mut() else {
            return MISS;
        };
        if self.rays_left == 0 {
            return MISS;
        }
        self.rays_left -= 1;
        let unit = direction.map(|v| v / length);
        world
            .raycast(origin, unit, max.min(MAX_RAY_DISTANCE))
            .filter(|d| d.is_finite())
            .unwrap_or(MISS)
    }

    /// Sets an input value readable by the script through `get_f32`.
    pub(crate) fn set_input(&mut self, name: &str, value: f32) {
        match self.inputs.get_mut(name) {
            Some(slot) => *slot = value,
            None => {
                self.inputs.insert(name.to_owned(), value);
            }
        }
    }

    /// Current input value, 0.0 when it was never set.
    pub(crate) fn input(&self, name: &str) -> f32 {
        self.inputs.get(name).copied().unwrap_or(0.0)
    }

    /// Stores an output; a new name is dropped once [`MAX_OUTPUTS`] names exist.
    pub(crate) fn set_output(&mut self, name: &str, value: f32) {
        if let Some(slot) = self.outputs.get_mut(name) {
            *slot = value;
        } else if self.outputs.len() < MAX_OUTPUTS {
            self.outputs.insert(name.to_owned(), value);
        }
    }

    /// Latest output stored under `name`.
    pub(crate) fn output(&self, name: &str) -> Option<f32> {
        self.outputs.get(name).copied()
    }

    /// Queues an event; dropped once [`MAX_EVENTS`] are pending.
    pub(crate) fn emit(&mut self, name: &str, value: f32) {
        if self.events.len() < MAX_EVENTS {
            self.events.push(ScriptEvent {
                name: name.to_owned(),
                value,
            });
        }
    }

    /// Removes and returns the queued events, oldest first.
    pub(crate) fn take_events(&mut self) -> Vec<ScriptEvent> {
        std::mem::take(&mut self.events)
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
    fn names_are_validated() {
        let mem = b"abc\xff";
        assert_eq!(read_name(mem, 0, 3), Some("abc"));
        assert_eq!(read_name(mem, 0, 4), None);
        assert_eq!(read_name(mem, 3, 5), None);
        assert_eq!(read_name(mem, -1, 1), None);
        assert_eq!(read_name(&[b'a'; 100], 0, 65), None);
        assert_eq!(read_name(mem, i32::MAX, i32::MAX), None);
    }

    struct Wall;
    impl WorldQueries for Wall {
        fn raycast(&mut self, _: [f32; 3], _: [f32; 3], _: f32) -> Option<f32> {
            Some(3.0)
        }
    }

    #[test]
    fn rays_need_a_world_valid_arguments_and_allowance() {
        let mut host = HostData::default();
        host.begin_call();
        let down = [0.0, -1.0, 0.0];
        assert_eq!(host.raycast([0.0; 3], down, 5.0), -1.0, "no world");
        host.set_world(Box::new(Wall));
        assert_eq!(host.raycast([0.0; 3], down, 5.0), 3.0);
        assert_eq!(host.raycast([f32::NAN, 0.0, 0.0], down, 5.0), -1.0);
        assert_eq!(host.raycast([0.0; 3], [0.0; 3], 5.0), -1.0);
        assert_eq!(host.raycast([0.0; 3], down, 0.0), -1.0);
        assert_eq!(host.raycast([0.0; 3], down, f32::NAN), -1.0);
        for _ in 0..MAX_RAYS_PER_CALL {
            host.raycast([0.0; 3], down, 5.0);
        }
        assert_eq!(host.raycast([0.0; 3], down, 5.0), -1.0, "allowance used up");
        host.begin_call();
        assert_eq!(host.raycast([0.0; 3], down, 5.0), 3.0, "renewed");
    }

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

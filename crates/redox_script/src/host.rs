//! Backend independent host state: the game clock and the bounded log buffer.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;

use crate::{ScriptCommand, ScriptEvent, WorldQueries};

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
/// Most commands queued per instance between two `take_commands` calls.
pub(crate) const MAX_COMMANDS: usize = 256;

/// Most `raycast` queries answered per call into the script.
pub(crate) const MAX_RAYS_PER_CALL: u32 = 64;
/// Longest ray the host is asked to cast, in metres.
pub(crate) const MAX_RAY_DISTANCE: f32 = 1000.0;
/// Most rays accepted by one `raycast_many` call.
pub(crate) const MAX_BATCH_RAYS: usize = 1024;
/// Bytes per ray record: seven little-endian `f32` (`ox, oy, oz, dx, dy, dz, max_distance`).
pub(crate) const RAY_RECORD_BYTES: usize = 28;
/// Bytes per answer: one little-endian `f32`.
const ANSWER_BYTES: usize = 4;

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

/// Validates a `raycast_many` call and returns the guest byte ranges `(records, answers)`.
///
/// `None` when `count` is outside `1..=MAX_BATCH_RAYS`, a pointer is negative, or either range
/// does not fit inside `memory_len`.
pub(crate) fn batch_ranges(
    memory_len: usize,
    rays_ptr: i32,
    count: i32,
    out_ptr: i32,
) -> Option<(Range<usize>, Range<usize>)> {
    let count = usize::try_from(count)
        .ok()
        .filter(|n| (1..=MAX_BATCH_RAYS).contains(n))?;
    let rays = guest_range(memory_len, rays_ptr, count * RAY_RECORD_BYTES)?;
    let out = guest_range(memory_len, out_ptr, count * ANSWER_BYTES)?;
    Some((rays, out))
}

/// `len` bytes starting at guest pointer `ptr`, when that range lies inside `memory_len`.
fn guest_range(memory_len: usize, ptr: i32, len: usize) -> Option<Range<usize>> {
    let start = usize::try_from(ptr).ok()?;
    let end = start.checked_add(len)?;
    (end <= memory_len).then_some(start..end)
}

/// Decodes one ray record; a short slice reads as zeros (callers pass whole records).
fn decode_ray(record: &[u8]) -> [f32; 7] {
    let mut ray = [0.0_f32; 7];
    for (value, bytes) in ray.iter_mut().zip(record.chunks_exact(4)) {
        let bytes: [u8; 4] = bytes.try_into().unwrap_or([0; 4]);
        *value = f32::from_le_bytes(bytes);
    }
    ray
}

/// Encodes answers as little-endian `f32` bytes, ready to be written to guest memory.
pub(crate) fn answer_bytes(answers: &[f32]) -> Vec<u8> {
    answers.iter().flat_map(|v| v.to_le_bytes()).collect()
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
    commands: Vec<ScriptCommand>,
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

    /// Answers `env.raycast_many` for the raw record bytes read from guest memory.
    ///
    /// Each record goes through [`HostData::raycast`], so the allowance and all checks apply per
    /// ray. `None` when the length is not a whole number of records or the count is outside
    /// `1..=MAX_BATCH_RAYS`.
    pub(crate) fn raycast_batch(&mut self, records: &[u8]) -> Option<Vec<f32>> {
        let chunks = records.chunks_exact(RAY_RECORD_BYTES);
        if !chunks.remainder().is_empty() {
            return None;
        }
        let count = chunks.len();
        if count == 0 || count > MAX_BATCH_RAYS {
            return None;
        }
        Some(
            chunks
                .map(|record| {
                    let [ox, oy, oz, dx, dy, dz, max] = decode_ray(record);
                    self.raycast([ox, oy, oz], [dx, dy, dz], max)
                })
                .collect(),
        )
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

    /// Copies every input set so far, so a replacement module can receive them.
    pub(crate) fn inputs_snapshot(&self) -> Vec<(String, f32)> {
        self.inputs
            .iter()
            .map(|(name, value)| (name.clone(), *value))
            .collect()
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

    /// Queues a command; dropped once [`MAX_COMMANDS`] are pending.
    pub(crate) fn emit_command(&mut self, name: &str, args: [f32; 3]) {
        if self.commands.len() < MAX_COMMANDS {
            self.commands.push(ScriptCommand {
                name: name.to_owned(),
                args,
            });
        }
    }

    /// Removes and returns the queued commands, oldest first.
    pub(crate) fn take_commands(&mut self) -> Vec<ScriptCommand> {
        std::mem::take(&mut self.commands)
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
    fn batch_ranges_check_count_and_bounds() {
        assert_eq!(batch_ranges(65_536, 64, 1, 512), Some((64..92, 512..516)));
        assert_eq!(
            batch_ranges(65_536, 0, 1024, 0).map(|(r, _)| r.len()),
            Some(28_672)
        );
        assert_eq!(batch_ranges(65_536, 0, 0, 0), None, "count 0");
        assert_eq!(batch_ranges(65_536, 0, -1, 0), None, "negative count");
        assert_eq!(
            batch_ranges(65_536, 0, 1025, 0),
            None,
            "count above the cap"
        );
        assert_eq!(
            batch_ranges(65_536, 65_530, 3, 0),
            None,
            "records leave memory"
        );
        assert_eq!(
            batch_ranges(65_536, 0, 3, 65_530),
            None,
            "answers leave memory"
        );
        assert_eq!(
            batch_ranges(65_536, -4, 3, 0),
            None,
            "negative records pointer"
        );
        assert_eq!(
            batch_ranges(65_536, 0, 3, -4),
            None,
            "negative answers pointer"
        );
        assert_eq!(batch_ranges(65_536, 0, 3, i32::MAX), None, "far outside");
    }

    #[test]
    fn batch_answers_follow_single_rays() {
        let mut host = HostData::default();
        host.begin_call();
        host.set_world(Box::new(Wall));
        let records = [
            ray_bytes([0.0, 2.0, 0.0, 0.0, -1.0, 0.0, 10.0]),
            ray_bytes([0.0; 7]),
        ];
        let answers = host.raycast_batch(&records.concat());
        assert_eq!(answers, Some(vec![3.0, -1.0]), "a zero direction misses");
    }

    #[test]
    fn batch_rejects_partial_records_and_bad_counts() {
        let mut host = HostData::default();
        host.begin_call();
        assert_eq!(host.raycast_batch(&[]), None, "no records");
        assert_eq!(host.raycast_batch(&[0; 27]), None, "short record");
        assert_eq!(host.raycast_batch(&[0; 29]), None, "trailing bytes");
        assert_eq!(
            host.raycast_batch(&vec![0; 28 * (MAX_BATCH_RAYS + 1)]),
            None,
            "too many"
        );
        assert!(
            host.raycast_batch(&vec![0; 28 * MAX_BATCH_RAYS]).is_some(),
            "the cap itself"
        );
    }

    #[test]
    fn batch_uses_the_per_call_allowance() {
        let mut host = HostData::default();
        host.begin_call();
        host.set_world(Box::new(Wall));
        let record = ray_bytes([0.0, 2.0, 0.0, 0.0, -1.0, 0.0, 10.0]);
        let records = record.repeat(MAX_RAYS_PER_CALL as usize + 1);
        let answers = host.raycast_batch(&records).unwrap_or_default();
        assert_eq!(answers.len(), MAX_RAYS_PER_CALL as usize + 1);
        assert_eq!(answers.last().copied(), Some(-1.0), "past the allowance");
    }

    #[test]
    fn answers_are_little_endian() {
        assert_eq!(
            answer_bytes(&[1.0, -1.0]),
            [1.0f32.to_le_bytes(), (-1.0f32).to_le_bytes()].concat()
        );
    }

    fn ray_bytes(ray: [f32; 7]) -> Vec<u8> {
        ray.iter().flat_map(|v| v.to_le_bytes()).collect()
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

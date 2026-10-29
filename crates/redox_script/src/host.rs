//! Backend independent host state: the game clock and the bounded log buffer.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;

use crate::{ScriptCommand, ScriptEntityId, ScriptEvent, WorldQueries};

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

/// Most world queries answered per call into the script. `raycast`, `raycast_entity`, the v2
/// lookups and overlaps all draw from this one allowance; further queries return their miss value.
pub(crate) const MAX_QUERIES_PER_CALL: u32 = 64;
/// Most entities returned by one overlap or tag search.
pub(crate) const MAX_OVERLAP_RESULTS: usize = 64;
/// Most points returned by one `nav_path` call.
pub(crate) const MAX_NAV_POINTS: usize = 32;
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
pub(crate) fn guest_range(memory_len: usize, ptr: i32, len: usize) -> Option<Range<usize>> {
    let start = usize::try_from(ptr).ok()?;
    let end = start.checked_add(len)?;
    (end <= memory_len).then_some(start..end)
}

/// Bytes of one entity id in guest memory (little-endian `u32`).
const ID_BYTES: usize = 4;
/// Bytes of one vector in guest memory (three little-endian `f32`).
pub(crate) const VEC3_BYTES: usize = 12;
/// Bytes of a `raycast_entity` answer: the id (`u32`) followed by the distance (`f32`).
pub(crate) const HIT_BYTES: usize = 8;

/// Bytes of one `f32` value written by `get_value_of`.
pub(crate) const VALUE_BYTES: usize = 4;

/// Guest byte range for an output buffer of up to `capacity` items of `item_bytes` each, and the
/// capacity actually granted: negative capacities count as zero, the grant is capped at
/// `max_items`.
fn buffer_range(
    memory_len: usize,
    out_ptr: i32,
    capacity: i32,
    item_bytes: usize,
    max_items: usize,
) -> Option<(Range<usize>, usize)> {
    let granted = usize::try_from(capacity).unwrap_or(0).min(max_items);
    let range = guest_range(memory_len, out_ptr, granted * item_bytes)?;
    Some((range, granted))
}

/// Guest byte range for an id buffer holding up to `capacity` ids (see [`buffer_range`]).
pub(crate) fn id_buffer_range(
    memory_len: usize,
    out_ptr: i32,
    capacity: i32,
) -> Option<(Range<usize>, usize)> {
    buffer_range(memory_len, out_ptr, capacity, ID_BYTES, MAX_OVERLAP_RESULTS)
}

/// Guest byte range for a path buffer holding up to `capacity` points (see [`buffer_range`]).
pub(crate) fn point_buffer_range(
    memory_len: usize,
    out_ptr: i32,
    capacity: i32,
) -> Option<(Range<usize>, usize)> {
    buffer_range(memory_len, out_ptr, capacity, VEC3_BYTES, MAX_NAV_POINTS)
}

/// Writes the ids into `memory` at `range` (which must hold at least `ids.len()` ids) and returns
/// how many were written, or 0 when the range does not fit the ids.
pub(crate) fn write_ids(memory: &mut [u8], range: Range<usize>, ids: &[u32]) -> i32 {
    let bytes: Vec<u8> = ids.iter().flat_map(|id| id.to_le_bytes()).collect();
    match memory.get_mut(range.start..range.start + bytes.len()) {
        Some(dst) => {
            dst.copy_from_slice(&bytes);
            i32::try_from(ids.len()).unwrap_or(0)
        }
        None => 0,
    }
}

/// Writes path points into `memory` at `range` and returns how many were written, or 0 when the
/// range does not fit them.
pub(crate) fn write_points(memory: &mut [u8], range: Range<usize>, points: &[[f32; 3]]) -> i32 {
    let bytes: Vec<u8> = points
        .iter()
        .flatten()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    match memory.get_mut(range.start..range.start + bytes.len()) {
        Some(dst) => {
            dst.copy_from_slice(&bytes);
            i32::try_from(points.len()).unwrap_or(0)
        }
        None => 0,
    }
}

/// Encodes a `raycast_entity` answer: the id, then the distance.
pub(crate) fn hit_bytes(id: u32, distance: f32) -> Vec<u8> {
    [id.to_le_bytes(), distance.to_le_bytes()].concat()
}

/// Reads the vector stored at guest pointer `ptr`; `None` when it does not fit in `memory`.
pub(crate) fn read_vec3(memory: &[u8], ptr: i32) -> Option<[f32; 3]> {
    let range = guest_range(memory.len(), ptr, VEC3_BYTES)?;
    let bytes = memory.get(range)?;
    let mut out = [0.0_f32; 3];
    for (value, chunk) in out.iter_mut().zip(bytes.chunks_exact(4)) {
        *value = f32::from_le_bytes(chunk.try_into().ok()?);
    }
    Some(out)
}

/// Converts an entity id to the `i32` a script receives: `-1` when it does not fit, which a
/// script reads as "not found" (ids are handed out sequentially and stay far below `i32::MAX`).
pub(crate) fn id_as_i32(id: u32) -> i32 {
    i32::try_from(id).unwrap_or(-1)
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

/// Checks a ray and normalises it: origin and direction must be finite, the direction non-zero
/// and `max` positive and finite. Returns the origin, the unit direction and the capped distance.
fn validate_ray(
    origin: [f32; 3],
    direction: [f32; 3],
    max: f32,
) -> Option<([f32; 3], [f32; 3], f32)> {
    let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !is_finite3(origin) || !is_finite3(direction) || !max.is_finite() || max <= 0.0 {
        return None;
    }
    if length <= 0.0 {
        return None;
    }
    Some((
        origin,
        direction.map(|v| v / length),
        max.min(MAX_RAY_DISTANCE),
    ))
}

/// True when all three components are finite numbers.
fn is_finite3(v: [f32; 3]) -> bool {
    v.iter().all(|c| c.is_finite())
}

/// Encodes answers as little-endian `f32` bytes, ready to be written to guest memory.
pub(crate) fn answer_bytes(answers: &[f32]) -> Vec<u8> {
    answers.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Prefix of the commands the host itself applies after a call (`sys.spawn`, ...). Other command
/// names belong to the game.
pub(crate) const SYSTEM_COMMAND_PREFIX: &str = "sys.";

/// Per-instance state shared between the host functions and the runtime.
#[derive(Debug, Default)]
pub(crate) struct HostData {
    /// Whether scripts may emit `sys.` commands.
    system_commands: bool,
    /// Accumulated game time in seconds.
    pub(crate) clock: f32,
    logs: VecDeque<String>,
    inputs: HashMap<String, f32>,
    outputs: HashMap<String, f32>,
    events: Vec<ScriptEvent>,
    commands: Vec<ScriptCommand>,
    world: WorldSlot,
    queries_left: u32,
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

    /// Starts a call into the script: the query allowance is renewed.
    pub(crate) fn begin_call(&mut self) {
        self.queries_left = MAX_QUERIES_PER_CALL;
    }

    /// Runs one world query against the installed world, if there is one and the allowance of
    /// the current call is not used up. `None` means the query was not answered.
    fn query<T>(&mut self, ask: impl FnOnce(&mut dyn WorldQueries) -> T) -> Option<T> {
        if self.queries_left == 0 {
            return None;
        }
        let world = self.world.0.as_mut()?;
        self.queries_left -= 1;
        Some(ask(world.as_mut()))
    }

    /// Answers `env.raycast`: the hit distance or -1.0 (no world, bad arguments, allowance used up,
    /// or no hit). The direction is normalised and the distance capped before the host is asked.
    pub(crate) fn raycast(&mut self, origin: [f32; 3], direction: [f32; 3], max: f32) -> f32 {
        const MISS: f32 = -1.0;
        let Some((origin, unit, max)) = validate_ray(origin, direction, max) else {
            return MISS;
        };
        self.query(|world| world.raycast(origin, unit, max))
            .flatten()
            .filter(|d| d.is_finite())
            .unwrap_or(MISS)
    }

    /// Answers the `raycast_entity` query: the first entity hit and the distance, or `None`.
    pub(crate) fn raycast_entity(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        max: f32,
    ) -> Option<(u32, f32)> {
        let (origin, unit, max) = validate_ray(origin, direction, max)?;
        self.query(|world| world.raycast_entity(origin, unit, max))
            .flatten()
            .filter(|(_, distance)| distance.is_finite())
            .map(|(id, distance)| (id.0, distance))
    }

    /// Entities whose volume overlaps the sphere, at most `limit` (and [`MAX_OVERLAP_RESULTS`]).
    /// Empty for a non finite centre, a negative or non finite radius, or when no query is left.
    pub(crate) fn overlap_sphere(
        &mut self,
        center: [f32; 3],
        radius: f32,
        limit: usize,
    ) -> Vec<u32> {
        if !is_finite3(center) || !radius.is_finite() || radius < 0.0 {
            return Vec::new();
        }
        self.collect_ids(limit, |world, out, cap| {
            world.overlap_sphere(center, radius, out, cap);
        })
    }

    /// Entities whose volume overlaps the box, at most `limit` (and [`MAX_OVERLAP_RESULTS`]).
    /// Empty for non finite or negative extents, or when no query is left.
    pub(crate) fn overlap_box(
        &mut self,
        center: [f32; 3],
        half_extent: [f32; 3],
        limit: usize,
    ) -> Vec<u32> {
        if !is_finite3(center) || !is_finite3(half_extent) || half_extent.iter().any(|h| *h < 0.0) {
            return Vec::new();
        }
        self.collect_ids(limit, |world, out, cap| {
            world.overlap_box(center, half_extent, out, cap);
        })
    }

    /// The entity with the given name, or `None` (no world, no query left, or unknown name).
    pub(crate) fn find_by_name(&mut self, name: &str) -> Option<u32> {
        self.query(|world| world.find_by_name(name))
            .flatten()
            .map(|id| id.0)
    }

    /// Entities carrying `tag`, at most `limit` (and [`MAX_OVERLAP_RESULTS`]).
    pub(crate) fn find_by_tag(&mut self, tag: &str, limit: usize) -> Vec<u32> {
        self.collect_ids(limit, |world, out, cap| world.find_by_tag(tag, out, cap))
    }

    /// World position of an entity; `None` when unknown, not finite, or no query is left.
    pub(crate) fn position_of(&mut self, id: u32) -> Option<[f32; 3]> {
        self.query(|world| world.position_of(ScriptEntityId(id)))
            .flatten()
            .filter(|p| is_finite3(*p))
    }

    /// Named numeric value of another entity; `None` when absent, not finite or no query is left.
    pub(crate) fn get_value(&mut self, id: u32, key: &str) -> Option<f32> {
        self.query(|world| world.get_value(ScriptEntityId(id), key))
            .flatten()
            .filter(|v| v.is_finite())
    }

    /// Walkable polyline from `from` to `to`, at most `limit` points (and [`MAX_NAV_POINTS`]).
    /// Empty when there is no path, a point is not finite, or no query is left.
    pub(crate) fn nav_path(&mut self, from: [f32; 3], to: [f32; 3], limit: usize) -> Vec<[f32; 3]> {
        if !is_finite3(from) || !is_finite3(to) {
            return Vec::new();
        }
        let cap = limit.min(MAX_NAV_POINTS);
        let mut path = Vec::new();
        let found = self
            .query(|world| world.nav_path(from, to, &mut path, cap))
            .unwrap_or(false);
        if !found || path.is_empty() || path.iter().any(|p| !is_finite3(*p)) {
            return Vec::new();
        }
        path.truncate(cap);
        path
    }

    /// Runs an id producing query and returns at most `limit` ids (never more than
    /// [`MAX_OVERLAP_RESULTS`]), even when the world returns more.
    fn collect_ids(
        &mut self,
        limit: usize,
        ask: impl FnOnce(&mut dyn WorldQueries, &mut Vec<ScriptEntityId>, usize),
    ) -> Vec<u32> {
        let cap = limit.min(MAX_OVERLAP_RESULTS);
        let mut found = Vec::new();
        if self.query(|world| ask(world, &mut found, cap)).is_none() {
            return Vec::new();
        }
        found.truncate(cap);
        found.into_iter().map(|id| id.0).collect()
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

    /// A host with the `sys.` command permission set as given (see
    /// [`ApiPermissions::commands_system`](crate::ApiPermissions::commands_system)).
    pub(crate) fn with_system_commands(allowed: bool) -> Self {
        Self {
            system_commands: allowed,
            ..Self::default()
        }
    }

    /// Queues a command; dropped once [`MAX_COMMANDS`] are pending. A `sys.` command is dropped
    /// (and logged) unless the system command permission was granted.
    pub(crate) fn emit_command(&mut self, name: &str, args: [f32; 3]) {
        if name.starts_with(SYSTEM_COMMAND_PREFIX) && !self.system_commands {
            log::warn!("script command `{name}` needs the system command permission");
            return;
        }
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
        for _ in 0..MAX_QUERIES_PER_CALL {
            host.raycast([0.0; 3], down, 5.0);
        }
        assert_eq!(host.raycast([0.0; 3], down, 5.0), -1.0, "allowance used up");
        host.begin_call();
        assert_eq!(host.raycast([0.0; 3], down, 5.0), 3.0, "renewed");
    }

    /// A world that returns far more entities than any query may hand back.
    struct Crowd;
    impl WorldQueries for Crowd {
        fn raycast(&mut self, _: [f32; 3], _: [f32; 3], _: f32) -> Option<f32> {
            Some(1.0)
        }

        fn overlap_sphere(&mut self, _: [f32; 3], _: f32, out: &mut Vec<ScriptEntityId>, _: usize) {
            out.extend((0..100).map(ScriptEntityId));
        }
    }

    #[test]
    fn the_overlap_limit_truncates_results() {
        let mut host = HostData::default();
        host.begin_call();
        host.set_world(Box::new(Crowd));
        assert_eq!(
            host.overlap_sphere([0.0; 3], 1.0, 1000).len(),
            MAX_OVERLAP_RESULTS
        );
        assert_eq!(host.overlap_sphere([0.0; 3], 1.0, 3), vec![0, 1, 2]);
        assert!(
            host.overlap_sphere([0.0; 3], -1.0, 3).is_empty(),
            "negative radius"
        );
        assert!(host.overlap_sphere([f32::NAN, 0.0, 0.0], 1.0, 3).is_empty());
    }

    #[test]
    fn world_queries_share_one_allowance_per_call() {
        let mut host = HostData::default();
        host.begin_call();
        host.set_world(Box::new(Crowd));
        for _ in 0..MAX_QUERIES_PER_CALL {
            host.overlap_sphere([0.0; 3], 1.0, 1);
        }
        assert!(host.overlap_sphere([0.0; 3], 1.0, 1).is_empty(), "used up");
        assert_eq!(
            host.raycast([0.0; 3], [0.0, -1.0, 0.0], 5.0),
            -1.0,
            "shared"
        );
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
        let records = record.repeat(MAX_QUERIES_PER_CALL as usize + 1);
        let answers = host.raycast_batch(&records).unwrap_or_default();
        assert_eq!(answers.len(), MAX_QUERIES_PER_CALL as usize + 1);
        assert_eq!(answers.last().copied(), Some(-1.0), "past the allowance");
    }

    #[test]
    fn answers_are_little_endian() {
        assert_eq!(
            answer_bytes(&[1.0, -1.0]),
            [1.0f32.to_le_bytes(), (-1.0f32).to_le_bytes()].concat()
        );
    }

    #[test]
    fn system_commands_require_the_permission() {
        let mut denied = HostData::default();
        denied.emit_command("sys.spawn", [0.0; 3]);
        denied.emit_command("game.fire", [1.0, 0.0, 0.0]);
        let names: Vec<String> = denied.take_commands().into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["game.fire"], "only game commands pass");

        let mut allowed = HostData::with_system_commands(true);
        allowed.emit_command("sys.spawn", [0.0; 3]);
        assert_eq!(allowed.take_commands().len(), 1);
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

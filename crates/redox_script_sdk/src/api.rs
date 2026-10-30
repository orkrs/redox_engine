//! Named values, events, commands and the single ray query that every script can use.

use crate::{addr_of_slice, host, len_of};

/// Longest name the host accepts, in bytes.
pub const MAX_NAME_BYTES: usize = 64;

/// Prefix the host reserves for system commands (`sys.spawn.<prefab>`, `sys.despawn.<id>`, ...).
const SYSTEM_PREFIX: &str = "sys.";

/// Reads the input `name` that the host set before this update. Unknown names read as `0.0`.
pub fn input(name: &str) -> f32 {
    // SAFETY: the host reads `name` during this call only; the slice outlives the call.
    unsafe { host::get_f32(addr_of_slice(name.as_bytes()), len_of(name.as_bytes())) }
}

/// Writes the output `name`; the host reads it back after the update. Invalid names are ignored.
pub fn output(name: &str, value: f32) {
    // SAFETY: as in `input`.
    unsafe { host::set_f32(addr_of_slice(name.as_bytes()), len_of(name.as_bytes()), value) }
}

/// Emits the event `name` with `value`. The host collects events of this call (at most 256).
pub fn emit(name: &str, value: f32) {
    // SAFETY: as in `input`.
    unsafe { host::emit_event(addr_of_slice(name.as_bytes()), len_of(name.as_bytes()), value) }
}

/// Emits the command `name` with three numbers. The host applies the commands after the call,
/// in the order they were emitted. Names starting with `sys.` are system commands.
pub fn command(name: &str, a: f32, b: f32, c: f32) {
    // SAFETY: as in `input`.
    unsafe {
        host::emit_command(
            addr_of_slice(name.as_bytes()),
            len_of(name.as_bytes()),
            a,
            b,
            c,
        )
    }
}

/// Emits the system command `sys.<operation>`, for example `system_command("spawn.crate", x, y, z)`.
/// Operations that do not fit in [`MAX_NAME_BYTES`] together with the prefix are not sent.
pub fn system_command(operation: &str, a: f32, b: f32, c: f32) {
    let mut buffer = [0u8; MAX_NAME_BYTES];
    let total = SYSTEM_PREFIX.len() + operation.len();
    if total > MAX_NAME_BYTES {
        return;
    }
    buffer[..SYSTEM_PREFIX.len()].copy_from_slice(SYSTEM_PREFIX.as_bytes());
    buffer[SYSTEM_PREFIX.len()..total].copy_from_slice(operation.as_bytes());
    // The prefix and the operation are both valid UTF-8, so the joined bytes are too.
    if let Ok(name) = core::str::from_utf8(&buffer[..total]) {
        command(name, a, b, c);
    }
}

/// Distance from `origin` along the unit-length `direction` to the first solid surface within
/// `max_distance`, or `None` when nothing is hit (also when the host has no world).
pub fn raycast(origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<f32> {
    // SAFETY: plain scalar arguments; the host answers -1.0 for a miss.
    let distance = unsafe {
        host::raycast(
            origin[0],
            origin[1],
            origin[2],
            direction[0],
            direction[1],
            direction[2],
            max_distance,
        )
    };
    (distance >= 0.0).then_some(distance)
}

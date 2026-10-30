//! Questions to the game world: entities by name or tag, positions, overlaps, values of other
//! entities, and navigation paths. Every answer is limited by the host (see `redox_script`).

use crate::{addr, addr_of_slice, host, len_of};

/// Id of an entity as scripts see it. It is not an ECS handle and is valid only as the host
/// reports it.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct EntityId(pub u32);

/// Layout of the record `raycast_entity` writes: the entity id, then the distance.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct Hit {
    id: u32,
    distance: f32,
}

/// Id of the entity named `name`, or `None` when no entity has that name.
pub fn find_by_name(name: &str) -> Option<EntityId> {
    // SAFETY: the host reads the name during this call only.
    let id = unsafe { host::find_by_name(addr_of_slice(name.as_bytes()), len_of(name.as_bytes())) };
    u32::try_from(id).ok().map(EntityId)
}

/// Writes the ids of entities tagged `tag` into `out` and returns how many were written
/// (at most `out.len()`).
pub fn find_by_tag(tag: &str, out: &mut [EntityId]) -> usize {
    // SAFETY: the host writes at most `out.len()` u32 values into `out`, which is valid for that.
    let written = unsafe {
        host::find_by_tag(
            addr_of_slice(tag.as_bytes()),
            len_of(tag.as_bytes()),
            addr_of_slice(out),
            len_of(out),
        )
    };
    usize::try_from(written).unwrap_or(0).min(out.len())
}

/// Position of entity `id`, or `None` when the entity has no position.
pub fn position_of(id: EntityId) -> Option<[f32; 3]> {
    let position = [0.0f32; 3];
    // SAFETY: the host writes three f32 values into `position`.
    let found = unsafe { host::position_of(id.0 as i32, addr_of_slice(&position)) };
    (found == 1).then_some(position)
}

/// First entity hit by the ray and the distance to it, or `None` on a miss.
pub fn raycast_entity(
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
) -> Option<(EntityId, f32)> {
    let mut hit = Hit::default();
    // SAFETY: the host reads two 12-byte vectors and writes one 8-byte `Hit` (`#[repr(C)]`).
    let found = unsafe {
        host::raycast_entity(
            addr_of_slice(&origin),
            addr_of_slice(&direction),
            max_distance,
            addr(&mut hit),
        )
    };
    (found == 1).then_some((EntityId(hit.id), hit.distance))
}

/// Writes the ids of the entities whose position lies within `radius` of `center` into `out`
/// and returns how many were written.
pub fn overlap_sphere(center: [f32; 3], radius: f32, out: &mut [EntityId]) -> usize {
    // SAFETY: the host writes at most `out.len()` u32 values into `out`.
    let written = unsafe {
        host::overlap_sphere(
            center[0],
            center[1],
            center[2],
            radius,
            addr_of_slice(out),
            len_of(out),
        )
    };
    usize::try_from(written).unwrap_or(0).min(out.len())
}

/// As [`overlap_sphere`], for an axis-aligned box given by its centre and half extents.
pub fn overlap_box(center: [f32; 3], half_extent: [f32; 3], out: &mut [EntityId]) -> usize {
    // SAFETY: as in `overlap_sphere`.
    let written = unsafe {
        host::overlap_box(
            center[0],
            center[1],
            center[2],
            half_extent[0],
            half_extent[1],
            half_extent[2],
            addr_of_slice(out),
            len_of(out),
        )
    };
    usize::try_from(written).unwrap_or(0).min(out.len())
}

/// Named value `key` of entity `id` (a field the entity publishes to scripts), or `None`.
pub fn get_value_of(id: EntityId, key: &str) -> Option<f32> {
    let mut value = 0.0f32;
    // SAFETY: the host reads the key during this call and writes one f32 into `value`.
    let found = unsafe {
        host::get_value_of(
            id.0 as i32,
            addr_of_slice(key.as_bytes()),
            len_of(key.as_bytes()),
            addr(&mut value),
        )
    };
    (found == 1).then_some(value)
}

/// Walkable path from `from` to `to` written into `out` as waypoints; returns the number of
/// waypoints (0 when there is no path or the host has no navigation data).
pub fn nav_path(from: [f32; 3], to: [f32; 3], out: &mut [[f32; 3]]) -> usize {
    // SAFETY: the host reads two vectors and writes at most `out.len()` points of 12 bytes.
    let written = unsafe {
        host::nav_path(
            addr_of_slice(&from),
            addr_of_slice(&to),
            addr_of_slice(out),
            len_of(out),
        )
    };
    usize::try_from(written).unwrap_or(0).min(out.len())
}

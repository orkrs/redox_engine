//! Shadow caster classification and moved-object detection for the VSM.
//!
//! Pure functions only (no GPU access), so they can be exercised without an adapter.

use std::collections::{HashMap, HashSet};

use redox_math::{Aabb, Mat4, Vec3};

/// How an object's shadow is cached by the virtual shadow map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowClass {
    /// Does not move after load; its pages stay cached.
    Static,
    /// Moves or deforms; its pages are refreshed when it moves.
    Dynamic,
    /// Old behaviour: decided from the size of the bounds (kept as the default so existing
    /// scenes look the same).
    #[default]
    Auto,
}

/// Rotation tolerance for the pose comparison: a basis column changing by more than this
/// (in model units) counts as a rotation of the object.
pub const SHADOW_ROTATION_EPS: f32 = 1.0e-3;

/// Translation tolerance (metres) used by the renderer when deciding whether a dynamic
/// caster moved enough to refresh its shadow pages.
pub const SHADOW_MOVED_EPS: f32 = 0.05;

/// Pose snapshot kept per dynamic caster: model matrix and world AABB at the moment its
/// shadow pages were last invalidated.
pub type ShadowPose = (Mat4, Aabb);

/// True when the object's shadow should be refreshed as it moves.
///
/// `Auto` uses the legacy size rule (not ground, and small in x/z and y).
pub fn is_dynamic_caster(class: ShadowClass, bounds: &Aabb) -> bool {
    match class {
        ShadowClass::Static => false,
        ShadowClass::Dynamic => true,
        ShadowClass::Auto => {
            let ext = bounds.max - bounds.min;
            !(ext.y < 0.6 && ext.x > 8.0 && ext.z > 8.0) // ground
                && ext.x < 6.0 && ext.z < 6.0 && ext.y < 4.0 // small dynamic
        }
    }
}

/// True when `cur` differs from `prev` by more than `eps` in translation, or by more than
/// [`SHADOW_ROTATION_EPS`] in any basis column.
fn pose_changed(prev: &Mat4, cur: &Mat4, eps: f32) -> bool {
    let moved: Vec3 = cur.w_axis.truncate() - prev.w_axis.truncate();
    if moved.length() > eps {
        return true;
    }
    (0..3).any(|axis| {
        let delta = cur.col(axis) - prev.col(axis);
        delta.truncate().length() > SHADOW_ROTATION_EPS
    })
}

/// AABBs whose shadow pages must be invalidated this frame.
///
/// `prev` holds the pose snapshot of each dynamic caster at its last invalidation, `cur` the
/// current `(id, model matrix, world AABB)` of every dynamic caster. Objects are matched by
/// `id`, so the order of `cur` does not matter.
///
/// - an object whose pose changed by more than `eps` (translation) since its snapshot yields
///   both its snapshot AABB and its current AABB;
/// - a new object (not in `prev`) yields its current AABB;
/// - an object that vanished (in `prev`, not in `cur`) yields its snapshot AABB.
///
/// Slow drift accumulates against the snapshot, so an object that creeps past `eps` over
/// several frames is still refreshed. Use [`next_shadow_poses`] to advance the snapshots.
pub fn moved_shadow_bounds(
    prev: &HashMap<u32, ShadowPose>,
    cur: &[(u32, Mat4, Aabb)],
    eps: f32,
) -> Vec<Aabb> {
    let mut out = Vec::new();
    let mut cur_ids = HashSet::with_capacity(cur.len());
    for (id, matrix, bounds) in cur {
        cur_ids.insert(*id);
        match prev.get(id) {
            None => out.push(*bounds),
            Some((prev_matrix, prev_bounds)) => {
                if pose_changed(prev_matrix, matrix, eps) {
                    out.push(*prev_bounds);
                    out.push(*bounds);
                }
            }
        }
    }
    for (id, (_, prev_bounds)) in prev {
        if !cur_ids.contains(id) {
            out.push(*prev_bounds);
        }
    }
    out
}

/// Snapshots to keep for the next frame.
///
/// An object that did not move past `eps` keeps its old snapshot (so slow drift keeps
/// accumulating); a moved or new object takes its current pose. Objects absent from `cur`
/// are dropped. Duplicate ids in `cur` are not supported: the last entry wins.
pub fn next_shadow_poses(
    prev: &HashMap<u32, ShadowPose>,
    cur: &[(u32, Mat4, Aabb)],
    eps: f32,
) -> HashMap<u32, ShadowPose> {
    cur.iter()
        .map(|(id, matrix, bounds)| {
            let kept = match prev.get(id) {
                Some((prev_matrix, prev_bounds)) if !pose_changed(prev_matrix, matrix, eps) => {
                    (*prev_matrix, *prev_bounds)
                }
                _ => (*matrix, *bounds),
            };
            (*id, kept)
        })
        .collect()
}

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

/// True when the object's shadow should be refreshed as it moves or deforms.
///
/// `Auto` uses the legacy size rule (not ground, and small in x/z and y), except that a
/// deforming object is always dynamic. An explicit `Static` wins over deformation: the game
/// has decided that the shadow does not change.
pub fn is_dynamic_caster(class: ShadowClass, bounds: &Aabb, deforms: bool) -> bool {
    match class {
        ShadowClass::Static => false,
        ShadowClass::Dynamic => true,
        ShadowClass::Auto => {
            if deforms {
                return true;
            }
            let ext = bounds.max - bounds.min;
            !(ext.y < 0.6 && ext.x > 8.0 && ext.z > 8.0) // ground
                && ext.x < 6.0 && ext.z < 6.0 && ext.y < 4.0 // small dynamic
        }
    }
}

/// Last-frame state of a deforming caster: its pose version and world AABB.
pub type DeformPose = (u32, Aabb);

/// AABBs whose shadow pages must be refreshed because a deforming caster changed its pose.
///
/// Unlike [`moved_shadow_bounds`], this compares against the previous frame, not a snapshot:
/// an animation that is running changes the pose every frame, so its old and new bounds are
/// both invalidated every frame. An object whose version is unchanged (paused animation,
/// same weights) invalidates nothing. A new object invalidates its current bounds. Objects
/// that vanished are handled by [`moved_shadow_bounds`], which already tracks them.
pub fn deformed_shadow_bounds(
    prev: &HashMap<u32, DeformPose>,
    cur: &[(u32, u32, Aabb)],
) -> Vec<Aabb> {
    let mut out = Vec::new();
    for (id, version, bounds) in cur {
        match prev.get(id) {
            None => out.push(*bounds),
            Some((prev_version, prev_bounds)) if prev_version != version => {
                out.push(*prev_bounds);
                out.push(*bounds);
            }
            Some(_) => {}
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(center: Vec3, half: f32) -> Aabb {
        Aabb::from_center_size(center, Vec3::splat(half))
    }

    fn pose(x: f32) -> (u32, Mat4, Aabb) {
        let center = Vec3::new(x, 0.5, 0.0);
        (1, Mat4::from_translation(center), cube(center, 0.5))
    }

    #[test]
    fn static_is_never_dynamic_even_when_small() {
        let small = cube(Vec3::ZERO, 0.5);
        assert!(!is_dynamic_caster(ShadowClass::Static, &small, false));
        assert!(!is_dynamic_caster(ShadowClass::Static, &small, true));
    }

    #[test]
    fn dynamic_is_dynamic_even_when_large() {
        let large = cube(Vec3::ZERO, 10.0);
        assert!(is_dynamic_caster(ShadowClass::Dynamic, &large, false));
    }

    #[test]
    fn auto_keeps_the_size_rule() {
        let ground = Aabb {
            min: Vec3::new(-10.0, 0.0, -10.0),
            max: Vec3::new(10.0, 0.3, 10.0),
        };
        let crate_box = cube(Vec3::ZERO, 0.5);
        let wide = Aabb {
            min: Vec3::new(-3.5, 0.0, -1.0),
            max: Vec3::new(3.5, 2.0, 1.0),
        };
        assert!(!is_dynamic_caster(ShadowClass::Auto, &ground, false));
        assert!(is_dynamic_caster(ShadowClass::Auto, &crate_box, false));
        assert!(!is_dynamic_caster(ShadowClass::Auto, &wide, false));
    }

    #[test]
    fn a_deforming_object_is_dynamic_regardless_of_size() {
        let large = cube(Vec3::ZERO, 10.0);
        assert!(is_dynamic_caster(ShadowClass::Auto, &large, true));
    }

    #[test]
    fn a_static_class_overrides_deformation() {
        let small = cube(Vec3::ZERO, 0.5);
        assert!(!is_dynamic_caster(ShadowClass::Static, &small, true));
    }

    #[test]
    fn an_unmoved_object_invalidates_nothing() {
        let (id, m, b) = pose(0.0);
        let prev = next_shadow_poses(&HashMap::new(), &[(id, m, b)], SHADOW_MOVED_EPS);
        assert!(moved_shadow_bounds(&prev, &[(id, m, b)], SHADOW_MOVED_EPS).is_empty());
    }

    #[test]
    fn a_moved_object_invalidates_old_and_new_bounds() {
        let (id, m0, b0) = pose(0.0);
        let prev = next_shadow_poses(&HashMap::new(), &[(id, m0, b0)], SHADOW_MOVED_EPS);
        let (_, m1, b1) = pose(3.0);
        let out = moved_shadow_bounds(&prev, &[(id, m1, b1)], SHADOW_MOVED_EPS);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&b0));
        assert!(out.contains(&b1));
    }

    #[test]
    fn slow_drift_accumulates_until_it_exceeds_the_threshold() {
        let (id, m0, b0) = pose(0.0);
        let mut prev = next_shadow_poses(&HashMap::new(), &[(id, m0, b0)], SHADOW_MOVED_EPS);
        let mut invalidations = Vec::new();
        for step in 1..=10 {
            let (_, m, b) = pose(0.01 * step as f32);
            let cur = [(id, m, b)];
            if !moved_shadow_bounds(&prev, &cur, 0.05).is_empty() {
                invalidations.push(step);
            }
            prev = next_shadow_poses(&prev, &cur, 0.05);
        }
        assert_eq!(invalidations, vec![6]);
    }

    #[test]
    fn a_removed_object_invalidates_its_last_bounds() {
        let (id, m, b) = pose(0.0);
        let prev = next_shadow_poses(&HashMap::new(), &[(id, m, b)], SHADOW_MOVED_EPS);
        let out = moved_shadow_bounds(&prev, &[], SHADOW_MOVED_EPS);
        assert_eq!(out, vec![b]);
    }

    #[test]
    fn reordering_the_list_does_not_invalidate() {
        let a = pose(0.0);
        let b = (
            2,
            Mat4::from_translation(Vec3::X * 5.0),
            cube(Vec3::X * 5.0, 0.5),
        );
        let prev = next_shadow_poses(&HashMap::new(), &[a, b], SHADOW_MOVED_EPS);
        assert!(moved_shadow_bounds(&prev, &[b, a], SHADOW_MOVED_EPS).is_empty());
    }

    #[test]
    fn an_unchanged_deform_version_invalidates_nothing() {
        let b = cube(Vec3::ZERO, 1.0);
        let prev: HashMap<u32, DeformPose> = [(7, (3, b))].into_iter().collect();
        assert!(deformed_shadow_bounds(&prev, &[(7, 3, b)]).is_empty());
    }

    #[test]
    fn a_changed_pose_invalidates_the_union_of_old_and_new_bounds() {
        let old = cube(Vec3::ZERO, 1.0);
        let new = cube(Vec3::new(0.5, 0.0, 0.0), 1.0);
        let prev: HashMap<u32, DeformPose> = [(7, (3, old))].into_iter().collect();
        let out = deformed_shadow_bounds(&prev, &[(7, 4, new)]);
        assert_eq!(out, vec![old, new]);
    }

    #[test]
    fn a_new_deforming_object_invalidates_its_bounds() {
        let b = cube(Vec3::ZERO, 1.0);
        assert_eq!(
            deformed_shadow_bounds(&HashMap::new(), &[(9, 0, b)]),
            vec![b]
        );
    }
}

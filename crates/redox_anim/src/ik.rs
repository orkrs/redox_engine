//! Analytic two-bone IK (leg / arm chains).

use redox_math::{Mat4, Quat, Vec3};

use crate::{LocalPose, Skeleton};

/// Smallest distance (relative to chain length) kept between root and target so the
/// law-of-cosines solution stays well-defined.
const MIN_REACH_EPSILON: f32 = 1.0e-4;

/// Solves a root–mid–effector chain so the effector reaches `target`.
///
/// All inputs are world-space joint positions. The mid joint bends toward `pole`.
/// Returns `(root_delta, mid_delta)` world-space rotations:
/// 1. rotate the whole chain about `root` by `root_delta`;
/// 2. then rotate the lower bone about the (already moved) mid joint by `mid_delta`.
///
/// Unreachable targets stretch the chain straight toward the target. Degenerate input
/// (zero-length bones) returns identity rotations.
pub fn solve_two_bone_ik(
    root: Vec3,
    mid: Vec3,
    effector: Vec3,
    target: Vec3,
    pole: Vec3,
) -> (Quat, Quat) {
    let upper = mid - root;
    let lower = effector - mid;
    let upper_len = upper.length();
    let lower_len = lower.length();
    if upper_len < f32::EPSILON || lower_len < f32::EPSILON {
        return (Quat::IDENTITY, Quat::IDENTITY);
    }

    let to_target = target - root;
    let target_dist = to_target.length();
    if target_dist < f32::EPSILON {
        return (Quat::IDENTITY, Quat::IDENTITY);
    }
    let axis = to_target / target_dist;

    // Clamp so |a-b| < d < a+b: the triangle always exists.
    let min_reach = (upper_len - lower_len).abs() + MIN_REACH_EPSILON;
    let max_reach = upper_len + lower_len - MIN_REACH_EPSILON;
    let reach = target_dist.clamp(min_reach.min(max_reach), max_reach);

    // Angle at the root between the root->target axis and the upper bone.
    let cos_root = ((upper_len * upper_len + reach * reach - lower_len * lower_len)
        / (2.0 * upper_len * reach))
        .clamp(-1.0, 1.0);
    let sin_root = (1.0 - cos_root * cos_root).sqrt();

    let bend_dir = bend_direction(axis, root, mid, pole);
    let new_mid = root + axis * (upper_len * cos_root) + bend_dir * (upper_len * sin_root);
    let new_effector = root + axis * reach;

    let root_delta = Quat::from_rotation_arc(upper / upper_len, (new_mid - root) / upper_len);
    let lower_after_root = root_delta * lower;
    let mid_delta = Quat::from_rotation_arc(
        lower_after_root / lower_len,
        (new_effector - new_mid) / lower_len,
    );
    (root_delta, mid_delta)
}

/// Bends `chain = [root, mid, tip]` (bone indices, `mid` a child of `root`) so the tip bone
/// reaches `target`. `target` and `pole` are model-space; only the rotations of `root` and
/// `mid` change in `local`.
pub fn apply_two_bone_ik(
    skeleton: &Skeleton,
    local: &mut LocalPose,
    chain: [usize; 3],
    target: Vec3,
    pole: Vec3,
) {
    let [root, mid, tip] = chain;
    let globals = skeleton.global_matrices(local);
    let position = |i: usize| globals[i].w_axis.truncate();
    let rotation = |m: Mat4| m.to_scale_rotation_translation().1;

    let (root_delta, mid_delta) =
        solve_two_bone_ik(position(root), position(mid), position(tip), target, pole);

    let parent_rotation = skeleton.bones[root]
        .parent
        .and_then(|p| globals.get(p))
        .map_or(Quat::IDENTITY, |m| rotation(*m));
    let new_root = root_delta * rotation(globals[root]);
    let new_mid = mid_delta * root_delta * rotation(globals[mid]);
    local.bones[root].rotation = (parent_rotation.inverse() * new_root).normalize();
    local.bones[mid].rotation = (new_root.inverse() * new_mid).normalize();
}

/// Unit vector perpendicular to `axis`, pointing toward the pole (or the current bend
/// when the pole is collinear with the axis).
fn bend_direction(axis: Vec3, root: Vec3, mid: Vec3, pole: Vec3) -> Vec3 {
    for hint in [pole - root, mid - root] {
        let perp = hint - axis * hint.dot(axis);
        if perp.length_squared() > 1.0e-10 {
            return perp.normalize();
        }
    }
    axis.any_orthonormal_vector()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies the solver output and returns the resulting (mid, effector).
    fn apply(root: Vec3, mid: Vec3, effector: Vec3, rot: (Quat, Quat)) -> (Vec3, Vec3) {
        let new_mid = root + rot.0 * (mid - root);
        let eff_after_root = root + rot.0 * (effector - root);
        let new_effector = new_mid + rot.1 * (eff_after_root - new_mid);
        (new_mid, new_effector)
    }

    const ROOT: Vec3 = Vec3::new(0.0, 2.0, 0.0);
    const MID: Vec3 = Vec3::new(0.0, 1.0, 0.1);
    const FOOT: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    const POLE_FRONT: Vec3 = Vec3::new(0.0, 1.0, 2.0);

    #[test]
    fn reaches_target_inside_range() {
        let target = Vec3::new(0.3, 0.4, 0.5);
        let rot = solve_two_bone_ik(ROOT, MID, FOOT, target, POLE_FRONT);
        let (_, effector) = apply(ROOT, MID, FOOT, rot);
        assert!(
            (effector - target).length() < 1.0e-3,
            "effector at {effector:?}"
        );
    }

    #[test]
    fn preserves_bone_lengths() {
        let target = Vec3::new(-0.5, 0.8, 0.4);
        let rot = solve_two_bone_ik(ROOT, MID, FOOT, target, POLE_FRONT);
        let (mid, effector) = apply(ROOT, MID, FOOT, rot);
        assert!(((mid - ROOT).length() - (MID - ROOT).length()).abs() < 1.0e-4);
        assert!(((effector - mid).length() - (FOOT - MID).length()).abs() < 1.0e-4);
    }

    #[test]
    fn bends_toward_pole() {
        let target = Vec3::new(0.0, 0.5, 0.0);
        let front = solve_two_bone_ik(ROOT, MID, FOOT, target, Vec3::new(0.0, 1.0, 5.0));
        let back = solve_two_bone_ik(ROOT, MID, FOOT, target, Vec3::new(0.0, 1.0, -5.0));
        let (mid_front, _) = apply(ROOT, MID, FOOT, front);
        let (mid_back, _) = apply(ROOT, MID, FOOT, back);
        assert!(mid_front.z > 0.0);
        assert!(mid_back.z < 0.0);
    }

    #[test]
    fn unreachable_target_stretches_toward_it() {
        let target = Vec3::new(0.0, 2.0, 10.0);
        let rot = solve_two_bone_ik(ROOT, MID, FOOT, target, POLE_FRONT);
        let (_, effector) = apply(ROOT, MID, FOOT, rot);
        let dir = (effector - ROOT).normalize();
        assert!(dir.dot((target - ROOT).normalize()) > 0.999);
        let chain = (MID - ROOT).length() + (FOOT - MID).length();
        assert!((effector - ROOT).length() <= chain + 1.0e-3);
    }

    #[test]
    fn degenerate_input_is_identity() {
        let rot = solve_two_bone_ik(ROOT, ROOT, FOOT, FOOT, POLE_FRONT);
        assert_eq!(rot, (Quat::IDENTITY, Quat::IDENTITY));
    }

    #[test]
    fn apply_moves_tip_bone_onto_target() {
        use crate::Pose;
        use crate::procedural::chain_model;

        let model = chain_model(2, 1.0, 0.1, 8);
        let mut local = model.skeleton.bind_local_pose();
        let target = Vec3::new(0.6, 1.1, 0.5);
        apply_two_bone_ik(
            &model.skeleton,
            &mut local,
            [0, 1, 2],
            target,
            Vec3::new(0.0, 1.0, 3.0),
        );
        let tip = Pose::from_local(&model.skeleton, &local).bone_transforms[2]
            .w_axis
            .truncate();
        assert!(
            tip.distance(target) < 1.0e-3,
            "tip at {tip:?}, wanted {target:?}"
        );
    }
}

//! Skeleton, clips and pose evaluation.
//!
//! Conventions:
//! * [`Bone::bind_pose`] is the bone's **local** bind transform (relative to its parent).
//! * [`Bone::inv_bind`] is the inverse of the bone's **model-space** bind matrix.
//! * [`Pose::bone_transforms`] hold **model-space** matrices.
//! * Clip tracks are keyed by bone name; keyframes must be sorted by time.

use redox_math::{Mat4, Quat, Vec3};

/// One joint of a skeleton.
#[derive(Clone, Debug)]
pub struct Bone {
    /// Name used to match animation tracks.
    pub name: String,
    /// Index of the parent bone, `None` for a root.
    pub parent: Option<usize>,
    /// Local bind transform (relative to the parent bone).
    pub bind_pose: Mat4,
    /// Inverse of the model-space bind matrix.
    pub inv_bind: Mat4,
}

/// Translation / rotation / scale of one bone, relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneTransform {
    /// Offset from the parent.
    pub translation: Vec3,
    /// Orientation relative to the parent.
    pub rotation: Quat,
    /// Scale relative to the parent.
    pub scale: Vec3,
}

impl BoneTransform {
    /// No translation, no rotation, unit scale.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Decomposes a matrix into translation, rotation and scale.
    pub fn from_mat4(m: Mat4) -> Self {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// Recomposes the matrix.
    pub fn to_mat4(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Linear blend (slerp for rotation); `t = 0` gives `self`, `t = 1` gives `other`.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// Per-bone local transforms (what clips produce and blends operate on).
#[derive(Clone, Debug, PartialEq)]
pub struct LocalPose {
    /// One transform per bone, in skeleton order.
    pub bones: Vec<BoneTransform>,
}

impl LocalPose {
    /// Blends two poses bone by bone. Extra bones in the longer pose are kept from `self`.
    pub fn blend(&self, other: &LocalPose, t: f32) -> LocalPose {
        let t = t.clamp(0.0, 1.0);
        let bones = self
            .bones
            .iter()
            .enumerate()
            .map(|(i, a)| other.bones.get(i).map_or(*a, |b| a.lerp(*b, t)))
            .collect();
        LocalPose { bones }
    }
}

/// A bone hierarchy with bind poses.
#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    /// Bones in an order where indices are stable (parents are referenced by index).
    pub bones: Vec<Bone>,
}

impl Skeleton {
    /// Builds a skeleton from `(name, parent, local bind transform)` triples.
    /// Bones may appear in any order; `inv_bind` is derived from the hierarchy.
    pub fn from_local_bind(bones: Vec<(String, Option<usize>, Mat4)>) -> Self {
        let mut skeleton = Skeleton {
            bones: bones
                .into_iter()
                .map(|(name, parent, bind_pose)| Bone {
                    name,
                    parent,
                    bind_pose,
                    inv_bind: Mat4::IDENTITY,
                })
                .collect(),
        };
        let globals = skeleton.global_matrices(&skeleton.bind_local_pose());
        for (bone, global) in skeleton.bones.iter_mut().zip(globals) {
            bone.inv_bind = global.inverse();
        }
        skeleton
    }

    /// Number of bones.
    pub fn len(&self) -> usize {
        self.bones.len()
    }

    /// Whether the skeleton has no bones.
    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    /// Index of the bone with this name.
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// Local bind pose (what unanimated bones fall back to).
    pub fn bind_local_pose(&self) -> LocalPose {
        LocalPose {
            bones: self
                .bones
                .iter()
                .map(|b| BoneTransform::from_mat4(b.bind_pose))
                .collect(),
        }
    }

    /// Model-space matrices for `local`. Works for any bone order; a parent cycle or an
    /// out-of-range parent index is treated as "no parent" instead of recursing forever.
    pub fn global_matrices(&self, local: &LocalPose) -> Vec<Mat4> {
        let mut out: Vec<Option<Mat4>> = vec![None; self.bones.len()];
        for i in 0..self.bones.len() {
            self.resolve_global(i, local, &mut out, 0);
        }
        out.into_iter()
            .map(|m| m.unwrap_or(Mat4::IDENTITY))
            .collect()
    }

    fn resolve_global(
        &self,
        i: usize,
        local: &LocalPose,
        memo: &mut [Option<Mat4>],
        depth: usize,
    ) -> Mat4 {
        if let Some(m) = memo[i] {
            return m;
        }
        let own = local.bones.get(i).map_or(Mat4::IDENTITY, |t| t.to_mat4());
        let parent = self.bones[i]
            .parent
            .filter(|&p| p < self.bones.len() && p != i);
        let global = match parent {
            Some(p) if depth < self.bones.len() => {
                self.resolve_global(p, local, memo, depth + 1) * own
            }
            _ => own,
        };
        memo[i] = Some(global);
        global
    }
}

/// A bone transform at a point in time.
#[derive(Clone, Debug)]
pub struct Keyframe {
    /// Time in seconds from the start of the clip.
    pub time: f32,
    /// Translation at this time.
    pub translation: Vec3,
    /// Rotation at this time.
    pub rotation: Quat,
    /// Scale at this time.
    pub scale: Vec3,
}

impl Keyframe {
    fn transform(&self) -> BoneTransform {
        BoneTransform {
            translation: self.translation,
            rotation: self.rotation,
            scale: self.scale,
        }
    }
}

/// Keyframed animation of a skeleton.
#[derive(Clone, Debug, Default)]
pub struct AnimationClip {
    /// Display name of the clip.
    pub name: String,
    /// Length in seconds.
    pub duration: f32,
    /// Keyframes per bone name, sorted by time.
    pub tracks: std::collections::HashMap<String, Vec<Keyframe>>,
    /// Keyframes of the morph weights (one value per target) sorted by time; empty = the clip
    /// does not animate morphs.
    pub morph_weights: Vec<(f32, Vec<f32>)>,
}

impl AnimationClip {
    /// Total number of keyframes over all tracks.
    pub fn keyframe_count(&self) -> usize {
        self.tracks.values().map(Vec::len).sum()
    }

    /// Morph weights at `time`: linear interpolation between neighbouring keyframes, clamped
    /// outside the clip and wrapped with `looping` (same time rules as [`Self::sample`]).
    /// `count` is the number of targets of the model; shorter keyframes are padded with 0.
    /// A clip without morph keyframes returns `count` zeros.
    pub fn sample_morph_weights(&self, time: f32, looping: bool, count: usize) -> Vec<f32> {
        let keys = &self.morph_weights;
        if keys.is_empty() {
            return vec![0.0; count];
        }
        let t = if looping && self.duration > f32::EPSILON {
            time.rem_euclid(self.duration)
        } else {
            time.clamp(0.0, self.duration.max(0.0))
        };
        let next = keys.partition_point(|k| k.0 <= t);
        if next == 0 {
            return padded(&keys[0].1, count);
        }
        if next >= keys.len() {
            return padded(&keys[keys.len() - 1].1, count);
        }
        let (a, b) = (&keys[next - 1], &keys[next]);
        let s = ((t - a.0) / (b.0 - a.0).max(f32::EPSILON)).clamp(0.0, 1.0);
        let (wa, wb) = (padded(&a.1, count), padded(&b.1, count));
        wa.iter().zip(&wb).map(|(x, y)| x + (y - x) * s).collect()
    }

    /// Samples the clip at `time`. With `looping` the time wraps, otherwise it clamps to the clip.
    /// Bones without a track keep their bind transform.
    pub fn sample(&self, skeleton: &Skeleton, time: f32, looping: bool) -> LocalPose {
        let t = if looping && self.duration > f32::EPSILON {
            time.rem_euclid(self.duration)
        } else {
            time.clamp(0.0, self.duration.max(0.0))
        };
        let bones = skeleton
            .bones
            .iter()
            .map(|bone| match self.tracks.get(&bone.name) {
                Some(keys) if !keys.is_empty() => sample_keys(keys, t),
                _ => BoneTransform::from_mat4(bone.bind_pose),
            })
            .collect();
        LocalPose { bones }
    }
}

/// Copies `values` into a vector of exactly `count` entries, padding with 0 or dropping the rest.
fn padded(values: &[f32], count: usize) -> Vec<f32> {
    (0..count)
        .map(|i| values.get(i).copied().unwrap_or(0.0))
        .collect()
}

fn sample_keys(keys: &[Keyframe], t: f32) -> BoneTransform {
    let next = keys.partition_point(|k| k.time <= t);
    if next == 0 {
        return keys[0].transform();
    }
    if next >= keys.len() {
        return keys[keys.len() - 1].transform();
    }
    let (a, b) = (&keys[next - 1], &keys[next]);
    let span = (b.time - a.time).max(f32::EPSILON);
    a.transform()
        .lerp(b.transform(), ((t - a.time) / span).clamp(0.0, 1.0))
}

/// Model-space bone matrices.
#[derive(Clone, Debug)]
pub struct Pose {
    /// Model-space matrix of every bone, in skeleton order.
    pub bone_transforms: Vec<Mat4>,
}

impl Pose {
    /// A pose where every bone matrix is the identity.
    pub fn identity(bone_count: usize) -> Self {
        Self {
            bone_transforms: vec![Mat4::IDENTITY; bone_count],
        }
    }

    /// Computes model-space matrices from local transforms by walking the hierarchy.
    pub fn from_local(skeleton: &Skeleton, local: &LocalPose) -> Self {
        Self {
            bone_transforms: skeleton.global_matrices(local),
        }
    }

    /// Skinning matrices (`model_space * inv_bind`) that move bind-pose vertices to this pose.
    pub fn skin_matrices(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        self.bone_transforms
            .iter()
            .zip(&skeleton.bones)
            .map(|(global, bone)| *global * bone.inv_bind)
            .collect()
    }
}

/// Component linking an entity to a skeleton and a clip by name.
#[derive(Clone, Debug)]
pub struct SkinnedMesh {
    /// Index of the skeleton.
    pub skeleton: usize,
    /// Name of the clip being played.
    pub clip: String,
    /// Playback position in seconds.
    pub time: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_bone() -> Skeleton {
        Skeleton::from_local_bind(vec![
            ("root".into(), None, Mat4::IDENTITY),
            ("tip".into(), Some(0), Mat4::from_translation(Vec3::Y)),
        ])
    }

    fn key(time: f32, rotation: Quat) -> Keyframe {
        Keyframe {
            time,
            translation: Vec3::Y,
            rotation,
            scale: Vec3::ONE,
        }
    }

    #[test]
    fn bind_pose_has_identity_skinning() {
        let skeleton = two_bone();
        let pose = Pose::from_local(&skeleton, &skeleton.bind_local_pose());
        for m in pose.skin_matrices(&skeleton) {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1.0e-5));
        }
    }

    #[test]
    fn child_follows_parent_rotation() {
        let skeleton = two_bone();
        let mut local = skeleton.bind_local_pose();
        local.bones[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let globals = skeleton.global_matrices(&local);
        let tip = globals[1].transform_point3(Vec3::ZERO);
        assert!(
            (tip - Vec3::new(-1.0, 0.0, 0.0)).length() < 1.0e-5,
            "tip at {tip:?}"
        );
    }

    #[test]
    fn global_matrices_accept_children_listed_before_parents() {
        let skeleton = Skeleton::from_local_bind(vec![
            ("tip".into(), Some(1), Mat4::from_translation(Vec3::Y)),
            ("root".into(), None, Mat4::from_translation(Vec3::X)),
        ]);
        let globals = skeleton.global_matrices(&skeleton.bind_local_pose());
        assert!(
            (globals[0].transform_point3(Vec3::ZERO) - Vec3::new(1.0, 1.0, 0.0)).length() < 1.0e-5
        );
    }

    #[test]
    fn parent_cycle_terminates() {
        let skeleton = Skeleton::from_local_bind(vec![
            ("a".into(), Some(1), Mat4::IDENTITY),
            ("b".into(), Some(0), Mat4::IDENTITY),
        ]);
        assert_eq!(
            skeleton.global_matrices(&skeleton.bind_local_pose()).len(),
            2
        );
    }

    #[test]
    fn clip_interpolates_between_keys_and_loops() {
        let skeleton = two_bone();
        let mut clip = AnimationClip {
            name: "c".into(),
            duration: 2.0,
            ..Default::default()
        };
        // 1.2 rad total: stays on the short arc, so the midpoint is unambiguous.
        clip.tracks.insert(
            "root".into(),
            vec![
                key(0.0, Quat::IDENTITY),
                key(2.0, Quat::from_rotation_z(1.2)),
            ],
        );
        let mid = clip.sample(&skeleton, 1.0, false).bones[0].rotation;
        assert!(mid.abs_diff_eq(Quat::from_rotation_z(0.6), 1.0e-4));
        let wrapped = clip.sample(&skeleton, 3.0, true).bones[0].rotation;
        assert!(wrapped.abs_diff_eq(mid, 1.0e-4), "t=3 wraps to t=1");
        let clamped = clip.sample(&skeleton, 9.0, false).bones[0].rotation;
        assert!(clamped.abs_diff_eq(Quat::from_rotation_z(1.2), 1.0e-4));
    }

    #[test]
    fn untracked_bones_keep_bind_pose() {
        let skeleton = two_bone();
        let clip = AnimationClip {
            name: "empty".into(),
            duration: 1.0,
            ..Default::default()
        };
        assert_eq!(
            clip.sample(&skeleton, 0.5, true),
            skeleton.bind_local_pose()
        );
    }

    #[test]
    fn blend_halfway_mixes_rotations() {
        let skeleton = two_bone();
        let a = skeleton.bind_local_pose();
        let mut b = a.clone();
        b.bones[0].rotation = Quat::from_rotation_z(1.0);
        let mid = a.blend(&b, 0.5);
        assert!(
            mid.bones[0]
                .rotation
                .abs_diff_eq(Quat::from_rotation_z(0.5), 1.0e-5)
        );
    }
}

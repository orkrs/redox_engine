//! Animation stack — skeleton, clips, inertialization, two-bone IK.

#![warn(missing_docs)]

pub mod animator;
pub mod gltf_import;
pub mod ik;
/// Hides the visible jump when switching clips by decaying the pose difference.
pub mod inertialization;
pub mod procedural;
pub mod skeleton;
pub mod skinning;

pub use animator::{Animator, SkinnedModel};
pub use ik::{apply_two_bone_ik, solve_two_bone_ik};
pub use inertialization::{InertialBoneDiff, Inertializer};
pub use skeleton::{
    AnimationClip, Bone, BoneTransform, Keyframe, LocalPose, Pose, Skeleton, SkinnedMesh,
};
pub use skinning::SkinnedGeometry;

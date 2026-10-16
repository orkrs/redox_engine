//! Animation stack — skeleton, clips, inertialization, two-bone IK.

pub mod ik;
pub mod inertialization;
pub mod skeleton;

pub use ik::solve_two_bone_ik;
pub use inertialization::{InertialBoneDiff, Inertializer};
pub use skeleton::{AnimationClip, Bone, Keyframe, Pose, Skeleton, SkinnedMesh};

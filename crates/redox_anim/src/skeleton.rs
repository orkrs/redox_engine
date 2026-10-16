use redox_math::{Mat4, Quat, Vec3};

#[derive(Clone, Debug)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub bind_pose: Mat4,
    pub inv_bind: Mat4,
}

#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

#[derive(Clone, Debug)]
pub struct Keyframe {
    pub time: f32,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

#[derive(Clone, Debug, Default)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub tracks: std::collections::HashMap<String, Vec<Keyframe>>,
}

#[derive(Clone, Debug)]
pub struct Pose {
    pub bone_transforms: Vec<Mat4>,
}

impl Pose {
    pub fn identity(bone_count: usize) -> Self {
        Self { bone_transforms: vec![Mat4::IDENTITY; bone_count] }
    }
}

#[derive(Clone, Debug)]
pub struct SkinnedMesh {
    pub skeleton: usize,
    pub clip: String,
    pub time: f32,
}

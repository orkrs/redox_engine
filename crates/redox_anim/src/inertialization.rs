use redox_math::{Quat, Vec3};

use crate::skeleton::Pose;

#[derive(Clone, Debug)]
pub struct InertialBoneDiff {
    pub x0: Vec3,
    pub v0: Vec3,
    pub q0: Quat,
    pub w0: Vec3,
}

#[derive(Clone, Debug)]
pub struct Inertializer {
    pub duration: f32,
    pub elapsed: f32,
    pub diffs: Vec<InertialBoneDiff>,
}

impl Default for Inertializer {
    fn default() -> Self {
        Self {
            duration: 0.15,
            elapsed: 0.0,
            diffs: Vec::new(),
        }
    }
}

impl Inertializer {
    pub fn new(duration: f32) -> Self {
        Self {
            duration,
            elapsed: 0.0,
            diffs: Vec::new(),
        }
    }

    pub fn record_transition(&mut self, old_pose: &Pose, new_pose: &Pose, dt: f32) {
        self.elapsed = 0.0;
        self.diffs.clear();
        let n = old_pose
            .bone_transforms
            .len()
            .min(new_pose.bone_transforms.len());
        for i in 0..n {
            let old_t = old_pose.bone_transforms[i].col(3).truncate();
            let new_t = new_pose.bone_transforms[i].col(3).truncate();
            let x0 = old_t - new_t;
            let v0 = if dt > 1e-6 { x0 / dt } else { Vec3::ZERO };
            self.diffs.push(InertialBoneDiff {
                x0,
                v0,
                q0: Quat::IDENTITY,
                w0: Vec3::ZERO,
            });
        }
    }

    pub fn apply_damping(&self, target_pose: &mut Pose, dt: f32) {
        if self.elapsed >= self.duration || self.diffs.is_empty() {
            return;
        }
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);
        // Hermite 5th poly with exp decay: (1 - t)^2 * (1 + 2t) * e^{-12t}
        let s = (1.0 - t).powi(2) * (1.0 + 2.0 * t) * (-12.0 * t).exp();
        for (i, d) in self.diffs.iter().enumerate() {
            if i >= target_pose.bone_transforms.len() {
                break;
            }
            let offset = d.x0 * s + d.v0 * dt * s;
            target_pose.bone_transforms[i].col_mut(3).x += offset.x;
            target_pose.bone_transforms[i].col_mut(3).y += offset.y;
            target_pose.bone_transforms[i].col_mut(3).z += offset.z;
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.elapsed = (self.elapsed + dt).min(self.duration);
    }

    pub fn is_active(&self) -> bool {
        self.elapsed < self.duration && !self.diffs.is_empty()
    }
}

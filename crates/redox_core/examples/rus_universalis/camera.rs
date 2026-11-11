//! Strategy camera: smooth pan, zoom, pitch and rotation above the strategic map.

#![allow(dead_code)]

use redox_math::{Mat4, Quat, Vec3, look_at, perspective};
use redox_render::camera::Camera;

#[derive(Clone, Debug)]
pub struct StrategyCamera {
    /// Look-at target center on the ground plane (Y = 0)
    pub target: Vec3,
    /// Distance from the target (zoom level)
    pub distance: f32,
    /// Tilt angle in radians (pitch above ground)
    pub pitch: f32,
    /// Rotation angle around Y in radians (yaw)
    pub yaw: f32,
    /// Target values for smooth interpolation
    target_pos: Vec3,
    target_dist: f32,
    target_yaw: f32,
}

impl StrategyCamera {
    pub fn new() -> Self {
        let initial_target = Vec3::new(0.5, 0.0, 0.5); // Center on Moscow/Vladimir
        Self {
            target: initial_target,
            distance: 14.0,
            pitch: 58.0_f32.to_radians(),
            yaw: 0.0,
            target_pos: initial_target,
            target_dist: 14.0,
            target_yaw: 0.0,
        }
    }

    pub fn pan(&mut self, delta_x: f32, delta_z: f32) {
        let rot = Quat::from_rotation_y(self.yaw);
        let move_vec = rot * Vec3::new(delta_x, 0.0, delta_z);
        self.target_pos += move_vec;
        // Clamp to map boundary
        self.target_pos.x = self.target_pos.x.clamp(-12.0, 12.0);
        self.target_pos.z = self.target_pos.z.clamp(-12.0, 12.0);
    }

    pub fn zoom(&mut self, scroll: f32) {
        self.target_dist = (self.target_dist - scroll * 1.5).clamp(4.5, 26.0);
    }

    pub fn rotate_yaw(&mut self, delta: f32) {
        self.target_yaw += delta;
    }

    /// Update smooth damping towards targets.
    pub fn update(&mut self, dt: f32) {
        let speed = (12.0 * dt).min(1.0);
        self.target = self.target.lerp(self.target_pos, speed);
        self.distance = self.distance + (self.target_dist - self.distance) * speed;
        self.yaw = self.yaw + (self.target_yaw - self.yaw) * speed;
    }

    /// Calculate eye position in world space.
    pub fn eye_position(&self) -> Vec3 {
        // Negative angle: positive `pitch` lifts the eye above the target (a positive rotation about X
        // would carry (0, 0, distance) below the ground and show the underside of the board).
        let pitch_rot = Quat::from_rotation_x(-self.pitch);
        let yaw_rot = Quat::from_rotation_y(self.yaw);
        let offset = yaw_rot * (pitch_rot * Vec3::new(0.0, 0.0, self.distance));
        self.target + offset
    }

    /// Build view matrix.
    pub fn view_matrix(&self) -> Mat4 {
        let eye = self.eye_position();
        look_at(eye, self.target, Vec3::Y)
    }

    /// Build projection matrix.
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        perspective(45.0_f32.to_radians(), aspect, 0.1, 150.0)
    }

    /// Build redox Camera instance.
    pub fn to_camera(&self, aspect: f32) -> Camera {
        Camera::new(45.0_f32.to_radians(), aspect, 0.1, 150.0)
    }
}

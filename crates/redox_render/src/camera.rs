//! Camera component and GPU uniform for the view-projection matrix.
//!
//! When TAA is enabled the [`RenderContext`] injects a subpixel jitter into
//! the projection matrix every frame (see [`crate::pass::taa`]).  The
//! unjittered VP is stored separately so the velocity pass can compute correct
//! motion vectors.

use bytemuck::{Pod, Zeroable};
use redox_ecs::Entity;
use redox_math::{Mat4, Quat, Vec3, look_at, perspective};

/// ECS component describing a perspective camera.
#[derive(Clone, Debug)]
pub struct Camera {
    /// Vertical field-of-view in **radians**.
    pub fov_y: f32,
    /// Near clipping plane distance.
    pub near: f32,
    /// Far clipping plane distance.
    pub far: f32,
    /// Aspect ratio (width / height). Updated on window resize.
    pub aspect_ratio: f32,
}

impl Camera {
    /// Creates a new camera with the given parameters.
    ///
    /// * `fov_y` — vertical field of view **in radians**.
    /// * `aspect_ratio` — width / height of the viewport.
    /// * `near` / `far` — clipping plane distances.
    pub fn new(fov_y: f32, aspect_ratio: f32, near: f32, far: f32) -> Self {
        Self {
            fov_y,
            aspect_ratio,
            near,
            far,
        }
    }

    /// Builds the projection matrix for this camera.
    #[inline]
    pub fn projection_matrix(&self) -> Mat4 {
        perspective(self.fov_y, self.aspect_ratio, self.near, self.far)
    }

    /// Builds a view matrix given the camera's world-space position and rotation.
    ///
    /// The camera looks along its local −Z axis (right-handed convention).
    #[inline]
    pub fn view_matrix(position: Vec3, rotation: Quat) -> Mat4 {
        let forward = rotation * Vec3::new(0.0, 0.0, -1.0);
        let target = position + forward;
        look_at(position, target, Vec3::Y)
    }

    /// Convenience: builds a combined view-projection matrix.
    pub fn view_proj_matrix(&self, position: Vec3, rotation: Quat) -> Mat4 {
        self.projection_matrix() * Self::view_matrix(position, rotation)
    }
}

/// ECS resource that identifies which entity is the active (main) camera.
#[derive(Clone, Copy, Debug)]
pub struct ActiveCamera(pub Entity);

/// GPU-friendly camera uniform.
///
/// Uploaded to a uniform buffer and bound in the vertex shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    /// Combined view-projection matrix (column-major).
    pub view_proj: [[f32; 4]; 4],
    /// Camera world-space position (padded to 16 bytes).
    pub camera_pos: [f32; 4],
}

impl CameraUniform {
    /// Creates a zeroed uniform (identity-like placeholder).
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0; 4],
        }
    }

    /// Updates the uniform from camera parameters and transform.
    pub fn update(&mut self, camera: &Camera, position: Vec3, rotation: Quat) {
        let vp = camera.view_proj_matrix(position, rotation);
        self.view_proj = vp.to_cols_array_2d();
        self.camera_pos = [position.x, position.y, position.z, 1.0];
    }
}

impl Default for CameraUniform {
    fn default() -> Self {
        Self::new()
    }
}

// ── ECS resource for TAA camera history ──────────────────────────────────────

/// ECS resource that stores the previous frame's **unjittered** VP matrix.
///
/// [`RenderContext`] writes this every frame so the velocity pass can compute
/// per-pixel motion vectors.  Applications never need to touch this directly.
#[derive(Clone, Debug)]
pub struct PreviousViewProj {
    /// Column-major 4×4 unjittered VP matrix from the last rendered frame.
    pub matrix: Mat4,
}

impl Default for PreviousViewProj {
    fn default() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }
}

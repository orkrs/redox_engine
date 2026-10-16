//! Camera component and GPU uniform.
//!
//! [`CameraUniform`] carries **separate** view / projection matrices plus their
//! inverses — this is what lets shadow passes (VSM) build correct clipmap
//! frustums from the real camera instead of a hardcoded guess.
//!
//! When TAA is enabled the renderer injects a subpixel jitter into the
//! projection matrix every frame (see [`crate::pass::taa`]). The unjittered
//! matrices are stored so the velocity pass can compute correct motion vectors.
//!
//! # GPU layout note
//! `view_proj` + `camera_pos` stay at offsets 0..80 for compatibility with
//! shaders compiled before the split; new fields are appended after them.

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

/// GPU-friendly camera uniform with separated matrices.
///
/// Uploaded to a uniform buffer; bound by every pass that needs camera data
/// (PBR, shadow, velocity, TAA, skybox).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    /// Combined view-projection matrix (column-major). Kept first for
    /// byte-compatibility with pre-split shaders.
    pub view_proj: [[f32; 4]; 4],
    /// Camera world-space position (padded to 16 bytes).
    pub camera_pos: [f32; 4],
    /// World→view matrix.
    pub view: [[f32; 4]; 4],
    /// View→clip matrix.
    pub proj: [[f32; 4]; 4],
    /// Inverse view matrix (clip-independent ray reconstruction).
    pub inv_view: [[f32; 4]; 4],
    /// Inverse projection matrix.
    pub inv_proj: [[f32; 4]; 4],
    /// Previous frame's **unjittered** view-projection (motion vectors).
    pub prev_view_proj: [[f32; 4]; 4],
}

impl CameraUniform {
    /// Creates a zeroed uniform (identity-like placeholder).
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0; 4],
            view: Mat4::IDENTITY.to_cols_array_2d(),
            proj: Mat4::IDENTITY.to_cols_array_2d(),
            inv_view: Mat4::IDENTITY.to_cols_array_2d(),
            inv_proj: Mat4::IDENTITY.to_cols_array_2d(),
            prev_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
        }
    }

    /// Updates all matrices from camera parameters and transform.
    ///
    /// `prev_view_proj` keeps its previous value (call
    /// [`snapshot_prev_view_proj`](Self::snapshot_prev_view_proj) at frame end).
    pub fn update(&mut self, camera: &Camera, position: Vec3, rotation: Quat) {
        let view = Camera::view_matrix(position, rotation);
        let proj = camera.projection_matrix();
        self.view = view.to_cols_array_2d();
        self.proj = proj.to_cols_array_2d();
        self.inv_view = view.inverse().to_cols_array_2d();
        self.inv_proj = proj.inverse().to_cols_array_2d();
        self.view_proj = (proj * view).to_cols_array_2d();
        self.camera_pos = [position.x, position.y, position.z, 1.0];
    }

    /// Copies the current combined VP into `prev_view_proj`.
    ///
    /// Call once at the end of each rendered frame (after velocity consumed
    /// the previous value).
    pub fn snapshot_prev_view_proj(&mut self) {
        self.prev_view_proj = self.view_proj;
    }

    /// Applies a subpixel NDC jitter to the projection and refreshes the
    /// combined `view_proj`. Used by TAA/TAAU before uploading the uniform.
    pub fn apply_jitter_ndc(&mut self, dx: f32, dy: f32) {
        let mut proj = Mat4::from_cols_array_2d(&self.proj);
        // Same derivation as the legacy combined-matrix path: shifting the
        // z-axis x/y components translates NDC xy.
        proj.z_axis.x -= dx;
        proj.z_axis.y -= dy;
        self.proj = proj.to_cols_array_2d();
        let view = Mat4::from_cols_array_2d(&self.view);
        self.view_proj = (proj * view).to_cols_array_2d();
    }

    /// Returns the current combined view-projection as a glam matrix.
    pub fn view_proj_matrix(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.view_proj)
    }

    /// Returns the previous (unjittered) view-projection as a glam matrix.
    pub fn prev_view_proj_matrix(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.prev_view_proj)
    }

    /// Returns the unjittered combined view-projection (recomputed from the
    /// stored view/proj pair, immune to per-frame jitter writes).
    pub fn unjittered_view_proj(&self, jitter_dx: f32, jitter_dy: f32) -> Mat4 {
        let mut proj = Mat4::from_cols_array_2d(&self.proj);
        proj.z_axis.x += jitter_dx;
        proj.z_axis.y += jitter_dy;
        proj * Mat4::from_cols_array_2d(&self.view)
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
/// [`RenderContext`](crate::RenderContext) writes this every frame so the
/// velocity pass can compute per-pixel motion vectors. Applications never need
/// to touch this directly. Prefer `CameraUniform::prev_view_proj` for new code.
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

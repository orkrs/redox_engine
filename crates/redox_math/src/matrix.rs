use crate::quat::Quat;
use crate::vector::Vec3;

/// A 4×4 column‑major matrix.
///
/// This is a re‑export of [`glam::Mat4`]. Use it for transformations,
/// projections, and view matrices.
pub type Mat4 = glam::Mat4;

/// A 3×3 column‑major matrix (normals, rotations without translation).
pub type Mat3 = glam::Mat3;

/// Creates a transformation matrix from translation, rotation, and scale.
///
/// The transformation order is: scale first, then rotation, then translation.
/// This matches the typical `T * R * S` convention used in many graphics
/// applications.
///
/// # Arguments
/// * `translation` - The translation vector.
/// * `rotation`    - The rotation quaternion.
/// * `scale`       - The scale factors.
///
/// # Example
/// ```
/// # use redox_math::{Vec3, Quat, transform_matrix};
/// let translation = Vec3::new(1.0, 2.0, 3.0);
/// let rotation = Quat::from_rotation_y(45.0_f32.to_radians());
/// let scale = Vec3::splat(2.0);
/// let mat = transform_matrix(translation, rotation, scale);
/// ```
#[inline]
pub fn transform_matrix(translation: Vec3, rotation: Quat, scale: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(scale, rotation, translation)
}

/// Creates a right‑handed view matrix (look‑at).
///
/// # Arguments
/// * `eye`    - The position of the camera.
/// * `target` - The point the camera is looking at.
/// * `up`     - The up direction (usually `Vec3::Y`).
///
/// # Returns
/// A matrix that transforms world space to view space (right‑handed).
#[inline]
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    Mat4::look_at_rh(eye, target, up)
}

/// Creates a right‑handed perspective projection matrix.
///
/// # Arguments
/// * `fov_radians`   - The vertical field of view in radians.
/// * `aspect_ratio`  - The aspect ratio (width / height).
/// * `near`          - Distance to the near clipping plane.
/// * `far`           - Distance to the far clipping plane.
///
/// # Returns
/// Creates a right‑handed perspective projection matrix (standard near->0, far->1).
#[inline]
pub fn perspective(fov_radians: f32, aspect_ratio: f32, near: f32, far: f32) -> Mat4 {
    // glam's `perspective_rh` already uses the WebGPU/wgpu [0, 1] depth range;
    // the GL-style [-1, 1] variant is `perspective_rh_gl`.
    Mat4::perspective_rh(fov_radians, aspect_ratio, near, far)
}

/// Creates a right‑handed Reverse-Z perspective projection matrix (near -> 1.0, far -> 0.0).
///
/// Reverse-Z vastly improves depth precision and eliminates Z-fighting at large distances
/// when paired with `wgpu::CompareFunction::GreaterEqual` and clearing depth to 0.0.
#[inline]
pub fn perspective_reverse_z(fov_radians: f32, aspect_ratio: f32, near: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_radians, aspect_ratio, far, near)
}

/// Creates a right‑handed Reverse-Z perspective projection matrix with infinite far plane.
#[inline]
pub fn perspective_infinite_reverse_z(fov_radians: f32, aspect_ratio: f32, near: f32) -> Mat4 {
    Mat4::perspective_infinite_reverse_rh(fov_radians, aspect_ratio, near)
}

/// Creates a right‑handed orthographic projection matrix.
///
/// # Arguments
/// * `left`   - Coordinate of the left clipping plane.
/// * `right`  - Coordinate of the right clipping plane.
/// * `bottom` - Coordinate of the bottom clipping plane.
/// * `top`    - Coordinate of the top clipping plane.
/// * `near`   - Distance to the near clipping plane.
/// * `far`    - Distance to the far clipping plane.
///
/// # Returns
/// An orthographic projection matrix (right‑handed, depth range [0,1]).
#[inline]
pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    Mat4::orthographic_rh(left, right, bottom, top, near, far)
}

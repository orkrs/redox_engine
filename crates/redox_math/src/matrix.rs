use crate::vector::Vec3;
use crate::quat::Quat;

/// 4x4 column-major matrix type alias
pub type Mat4 = glam::Mat4;

/// Creates a transformation matrix from translation, rotation, and scale.
///
/// The resulting matrix combines: T * R * S (scale first, then rotation, then translation)
///
/// # Arguments
/// * `translation` - The translation vector
/// * `rotation` - The rotation quaternion
/// * `scale` - The scale vector
///
/// # Returns
/// A 4x4 transformation matrix
#[inline]
pub fn transform_matrix(translation: Vec3, rotation: Quat, scale: Vec3) -> Mat4 {
    Mat4::from_scale_rotation_translation(scale, rotation, translation)
}

/// Creates a view matrix looking at a target.
///
/// # Arguments
/// * `eye` - The position of the camera
/// * `target` - The position to look at
/// * `up` - The up direction
///
/// # Returns
/// A view matrix (right-handed)
#[inline]
pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    Mat4::look_at_rh(eye, target, up)
}

/// Creates a perspective projection matrix.
///
/// # Arguments
/// * `fov_radians` - Vertical field of view in radians
/// * `aspect_ratio` - Width / Height
/// * `near` - Near plane distance
/// * `far` - Far plane distance
///
/// # Returns
/// A perspective projection matrix (right-handed, reversed Z)
#[inline]
pub fn perspective(fov_radians: f32, aspect_ratio: f32, near: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_radians, aspect_ratio, near, far)
}

/// Creates an orthographic projection matrix.
///
/// # Arguments
/// * `left` - Left plane
/// * `right` - Right plane
/// * `bottom` - Bottom plane
/// * `top` - Top plane
/// * `near` - Near plane
/// * `far` - Far plane
///
/// # Returns
/// An orthographic projection matrix (right-handed)
#[inline]
pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    Mat4::orthographic_rh(left, right, bottom, top, near, far)
}

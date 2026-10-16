use crate::vector::Vec3;

/// A quaternion representing a rotation.
///
/// This is a re‑export of [`glam::Quat`]. Quaternions are used for smooth
/// interpolation and to avoid gimbal lock.
pub type Quat = glam::Quat;

/// Returns the identity quaternion (no rotation).
#[inline]
pub fn identity() -> Quat {
    Quat::IDENTITY
}

/// Creates a quaternion from an axis and an angle (in radians).
///
/// The axis must be a unit vector.
///
/// # Arguments
/// * `axis`  - The rotation axis (normalized).
/// * `angle` - The rotation angle in radians.
///
/// # Example
/// ```
/// # use redox_math::{Vec3, from_axis_angle};
/// let rotation = from_axis_angle(Vec3::Y, 90.0_f32.to_radians());
/// ```
#[inline]
pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
    Quat::from_axis_angle(axis, angle)
}

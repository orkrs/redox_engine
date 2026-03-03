use crate::vector::Vec3;

/// Quaternion type alias
pub type Quat = glam::Quat;

/// Returns the identity quaternion (no rotation).
#[inline]
pub fn identity() -> Quat {
    Quat::IDENTITY
}

/// Creates a quaternion from an axis and angle (in radians).
///
/// # Arguments
/// * `axis` - The rotation axis (should be normalized)
/// * `angle` - The rotation angle in radians
///
/// # Returns
/// A quaternion representing the rotation
#[inline]
pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
    Quat::from_axis_angle(axis, angle)
}

/// Creates a quaternion from Euler angles (XYZ order, in radians).
///
/// # Arguments
/// * `x` - Rotation around X axis in radians
/// * `y` - Rotation around Y axis in radians
/// * `z` - Rotation around Z axis in radians
///
/// # Returns
/// A quaternion representing the combined rotation
#[inline]
pub fn from_euler_angles(x: f32, y: f32, z: f32) -> Quat {
    Quat::from_euler(glam::EulerRot::XYZ, x, y, z)
}

/// Creates a quaternion that rotates from one direction to another.
///
/// # Arguments
/// * `from` - The starting direction (should be normalized)
/// * `to` - The target direction (should be normalized)
///
/// # Returns
/// A quaternion representing the rotation from `from` to `to`
#[inline]
pub fn from_rotation_arc(from: Vec3, to: Vec3) -> Quat {
    Quat::from_rotation_arc(from, to)
}

/// Spherically interpolates between two quaternions.
///
/// # Arguments
/// * `a` - Starting quaternion
/// * `b` - Target quaternion
/// * `t` - Interpolation factor (0.0 to 1.0)
///
/// # Returns
/// The interpolated quaternion
#[inline]
pub fn slerp(a: Quat, b: Quat, t: f32) -> Quat {
    a.slerp(b, t)
}

/// Rotates a vector by a quaternion.
///
/// # Arguments
/// * `quat` - The rotation quaternion
/// * `vec` - The vector to rotate
///
/// # Returns
/// The rotated vector
#[inline]
pub fn rotate_vec3(quat: Quat, vec: Vec3) -> Vec3 {
    quat * vec
}

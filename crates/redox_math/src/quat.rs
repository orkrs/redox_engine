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

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

    /// Absolute tolerance used for all float comparisons in this module.
    const EPS: f32 = 1e-4;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() <= EPS
    }

    fn assert_vec3_approx(actual: Vec3, expected: Vec3) {
        assert!(
            approx_eq(actual.x, expected.x)
                && approx_eq(actual.y, expected.y)
                && approx_eq(actual.z, expected.z),
            "expected {expected:?}, got {actual:?}"
        );
    }

    fn assert_quat_same_rotation(actual: Quat, expected: Quat) {
        // q and -q encode the same rotation, so compare the absolute dot product.
        assert!(
            approx_eq(actual.dot(expected).abs(), 1.0),
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn identity_returns_the_identity_quaternion() {
        assert_eq!(identity(), Quat::IDENTITY);
    }

    #[test]
    fn identity_leaves_vectors_unchanged() {
        let v = Vec3::new(1.5, -2.0, 3.0);
        assert_vec3_approx(identity() * v, v);
    }

    #[test]
    fn rotating_x_axis_90_degrees_about_z_gives_y_axis() {
        let q = from_axis_angle(Vec3::Z, FRAC_PI_2);
        assert_vec3_approx(q * Vec3::X, Vec3::Y);
    }

    #[test]
    fn rotating_y_axis_90_degrees_about_x_gives_z_axis() {
        let q = from_axis_angle(Vec3::X, FRAC_PI_2);
        assert_vec3_approx(q * Vec3::Y, Vec3::Z);
    }

    #[test]
    fn negative_angle_rotates_the_other_way() {
        let q = from_axis_angle(Vec3::Z, -FRAC_PI_2);
        assert_vec3_approx(q * Vec3::X, Vec3::new(0.0, -1.0, 0.0));
    }

    #[test]
    fn full_turn_returns_vector_to_its_starting_point() {
        let q = from_axis_angle(Vec3::new(1.0, 2.0, 3.0).normalize(), TAU);
        let v = Vec3::new(0.3, -0.7, 1.1);
        assert_vec3_approx(q * v, v);
    }

    #[test]
    fn composition_applies_right_operand_first() {
        // Y -> (about X by 90, the right operand) -> Z -> (about Z by 90) -> Z.
        let rot_z = from_axis_angle(Vec3::Z, FRAC_PI_2);
        let rot_x = from_axis_angle(Vec3::X, FRAC_PI_2);
        assert_vec3_approx((rot_z * rot_x) * Vec3::Y, Vec3::Z);
    }

    #[test]
    fn composition_order_matters_for_non_commuting_rotations() {
        let rot_z = from_axis_angle(Vec3::Z, FRAC_PI_2);
        let rot_x = from_axis_angle(Vec3::X, FRAC_PI_2);
        // Reversed order: Y -> (about Z 90) -> -X -> (about X 90) -> -X.
        assert_vec3_approx((rot_x * rot_z) * Vec3::Y, Vec3::new(-1.0, 0.0, 0.0));
    }

    #[test]
    fn slerp_at_zero_returns_start_rotation() {
        let a = from_axis_angle(Vec3::Y, 0.4);
        let b = from_axis_angle(Vec3::X, 1.2);
        assert_quat_same_rotation(a.slerp(b, 0.0), a);
    }

    #[test]
    fn slerp_at_one_returns_end_rotation() {
        let a = from_axis_angle(Vec3::Y, 0.4);
        let b = from_axis_angle(Vec3::X, 1.2);
        assert_quat_same_rotation(a.slerp(b, 1.0), b);
    }

    #[test]
    fn slerp_midpoint_rotates_by_half_the_angle() {
        let end = from_axis_angle(Vec3::Z, FRAC_PI_2);
        let mid = identity().slerp(end, 0.5);
        assert_quat_same_rotation(mid, from_axis_angle(Vec3::Z, FRAC_PI_4));
    }

    #[test]
    fn slerp_takes_the_shortest_path_when_dot_product_is_negative() {
        // Identity to a -90 degree rotation expressed as a 270 degree one: the
        // quaternion dot product is negative, but the short way is through -45 degrees.
        let end = from_axis_angle(Vec3::Z, 3.0 * FRAC_PI_2);
        assert!(identity().dot(end) < 0.0);

        let mid = identity().slerp(end, 0.5);
        assert_vec3_approx(
            mid * Vec3::X,
            Vec3::new(0.5_f32.sqrt(), -0.5_f32.sqrt(), 0.0),
        );
    }

    #[test]
    fn slerp_between_nearly_identical_rotations_stays_finite_and_close() {
        let a = identity();
        let b = from_axis_angle(Vec3::Z, 1e-7);
        let mid = a.slerp(b, 0.5);
        assert!(mid.is_finite());
        assert_quat_same_rotation(mid, identity());
    }

    #[test]
    fn normalising_a_tiny_quaternion_yields_unit_length() {
        let tiny = Quat::from_xyzw(1e-4, 0.0, 0.0, 0.0);
        let unit = tiny.normalize();
        assert!(unit.is_finite());
        assert!(approx_eq(unit.length(), 1.0));
        assert_vec3_approx(Vec3::new(unit.x, unit.y, unit.z), Vec3::X);
    }

    #[test]
    fn half_turn_quaternion_rotates_by_pi() {
        let q = from_axis_angle(Vec3::Y, PI);
        assert_vec3_approx(q * Vec3::X, Vec3::new(-1.0, 0.0, 0.0));
    }
}

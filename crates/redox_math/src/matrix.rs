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
    glam::camera::rh::view::look_at_mat4(eye, target, up)
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
    glam::camera::rh::proj::directx::perspective(fov_radians, aspect_ratio, near, far)
}

/// Creates a right‑handed Reverse-Z perspective projection matrix (near -> 1.0, far -> 0.0).
///
/// Reverse-Z vastly improves depth precision and eliminates Z-fighting at large distances
/// when paired with `wgpu::CompareFunction::GreaterEqual` and clearing depth to 0.0.
#[inline]
pub fn perspective_reverse_z(fov_radians: f32, aspect_ratio: f32, near: f32, far: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective(fov_radians, aspect_ratio, far, near)
}

/// Creates a right‑handed Reverse-Z perspective projection matrix with infinite far plane.
#[inline]
pub fn perspective_infinite_reverse_z(fov_radians: f32, aspect_ratio: f32, near: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective_infinite_reverse(fov_radians, aspect_ratio, near)
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
/// An orthographic projection matrix (right‑handed, depth range \[0,1\]).
#[inline]
pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Mat4 {
    glam::camera::rh::proj::directx::orthographic(left, right, bottom, top, near, far)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_axis_angle;
    use std::f32::consts::FRAC_PI_2;

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

    fn assert_mat4_approx(actual: Mat4, expected: Mat4) {
        for (a, e) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!(approx_eq(a, e), "expected {expected:?}, got {actual:?}");
        }
    }

    fn sample_rotation() -> Quat {
        from_axis_angle(Vec3::new(1.0, 1.0, 0.0).normalize(), 0.7)
    }

    #[test]
    fn transform_matrix_with_identity_parts_is_identity() {
        let m = transform_matrix(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        assert_mat4_approx(m, Mat4::IDENTITY);
    }

    #[test]
    fn transform_matrix_moves_origin_to_translation() {
        let t = Vec3::new(1.0, 2.0, 3.0);
        let m = transform_matrix(t, sample_rotation(), Vec3::splat(2.0));
        assert_vec3_approx(m.transform_point3(Vec3::ZERO), t);
    }

    #[test]
    fn transform_matrix_applies_scale_then_rotation_then_translation() {
        // (1,0,0) -> scale x2 on X -> (2,0,0) -> rotate 90 about Z -> (0,2,0) -> translate -> (10,2,0)
        let m = transform_matrix(
            Vec3::new(10.0, 0.0, 0.0),
            from_axis_angle(Vec3::Z, FRAC_PI_2),
            Vec3::new(2.0, 1.0, 1.0),
        );
        assert_vec3_approx(m.transform_point3(Vec3::X), Vec3::new(10.0, 2.0, 0.0));
    }

    #[test]
    fn transform_matrix_round_trips_through_decomposition() {
        let t = Vec3::new(1.0, -2.0, 3.0);
        let r = sample_rotation();
        let s = Vec3::new(1.0, 2.0, 3.0);

        let (scale, rotation, translation) =
            transform_matrix(t, r, s).to_scale_rotation_translation();

        assert_vec3_approx(scale, s);
        assert_vec3_approx(translation, t);
        // q and -q describe the same rotation, so compare |dot|.
        assert!(
            approx_eq(rotation.dot(r).abs(), 1.0),
            "rotation differs: {rotation:?}"
        );
    }

    #[test]
    fn transform_matrix_times_its_inverse_is_identity() {
        let m = transform_matrix(
            Vec3::new(4.0, -1.0, 2.0),
            sample_rotation(),
            Vec3::new(1.0, 2.0, 0.5),
        );
        assert_mat4_approx(m.inverse() * m, Mat4::IDENTITY);
    }

    #[test]
    fn zero_scale_collapses_every_point_onto_translation() {
        let t = Vec3::new(1.0, 2.0, 3.0);
        let m = transform_matrix(t, sample_rotation(), Vec3::ZERO);

        assert!(approx_eq(m.determinant(), 0.0));
        assert_vec3_approx(m.transform_point3(Vec3::new(5.0, 6.0, 7.0)), t);
    }

    #[test]
    fn look_at_maps_eye_to_view_origin() {
        let eye = Vec3::new(0.0, 0.0, 5.0);
        let view = look_at(eye, Vec3::ZERO, Vec3::Y);
        assert_vec3_approx(view.transform_point3(eye), Vec3::ZERO);
    }

    #[test]
    fn look_at_puts_target_on_negative_z_axis_in_right_handed_view_space() {
        let view = look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        assert_vec3_approx(view.transform_point3(Vec3::ZERO), Vec3::new(0.0, 0.0, -5.0));
    }

    #[test]
    fn look_at_inverse_maps_view_origin_back_to_eye() {
        let eye = Vec3::new(3.0, 4.0, 5.0);
        let view = look_at(eye, Vec3::new(0.0, 1.0, 0.0), Vec3::Y);
        assert_vec3_approx(view.inverse().transform_point3(Vec3::ZERO), eye);
    }

    #[test]
    fn perspective_maps_near_plane_to_depth_zero_and_far_plane_to_one() {
        let m = perspective(FRAC_PI_2, 1.0, 1.0, 100.0);
        assert!(approx_eq(
            m.project_point3(Vec3::new(0.0, 0.0, -1.0)).z,
            0.0
        ));
        assert!(approx_eq(
            m.project_point3(Vec3::new(0.0, 0.0, -100.0)).z,
            1.0
        ));
    }

    #[test]
    fn perspective_centre_ray_maps_to_screen_centre() {
        let m = perspective(FRAC_PI_2, 16.0 / 9.0, 0.1, 1000.0);
        let ndc = m.project_point3(Vec3::new(0.0, 0.0, -10.0));
        assert!(approx_eq(ndc.x, 0.0) && approx_eq(ndc.y, 0.0));
    }

    #[test]
    fn perspective_fov_and_aspect_edges_map_to_ndc_edges() {
        // 90 degree vertical fov gives focal length 1; aspect 2 means the
        // half-width at depth 1 is 2, so (2, 1, -1) sits on the top-right corner.
        let m = perspective(FRAC_PI_2, 2.0, 1.0, 100.0);
        let ndc = m.project_point3(Vec3::new(2.0, 1.0, -1.0));
        assert!(approx_eq(ndc.x, 1.0), "x: {}", ndc.x);
        assert!(approx_eq(ndc.y, 1.0), "y: {}", ndc.y);
    }

    #[test]
    fn reverse_z_perspective_maps_near_plane_to_one_and_far_plane_to_zero() {
        let m = perspective_reverse_z(FRAC_PI_2, 1.0, 1.0, 100.0);
        assert!(approx_eq(
            m.project_point3(Vec3::new(0.0, 0.0, -1.0)).z,
            1.0
        ));
        assert!(approx_eq(
            m.project_point3(Vec3::new(0.0, 0.0, -100.0)).z,
            0.0
        ));
    }

    #[test]
    fn infinite_reverse_z_maps_near_plane_to_one_and_depth_is_inverse_distance() {
        let m = perspective_infinite_reverse_z(FRAC_PI_2, 1.0, 1.0);
        assert!(approx_eq(
            m.project_point3(Vec3::new(0.0, 0.0, -1.0)).z,
            1.0
        ));
        // depth = near / distance, so twice the near distance gives one half.
        assert!(approx_eq(
            m.project_point3(Vec3::new(0.0, 0.0, -2.0)).z,
            0.5
        ));
    }

    #[test]
    fn orthographic_maps_view_box_corners_to_ndc_corners() {
        let m = orthographic(-2.0, 2.0, -1.0, 1.0, 1.0, 11.0);
        assert_vec3_approx(
            m.project_point3(Vec3::new(2.0, 1.0, -1.0)),
            Vec3::new(1.0, 1.0, 0.0),
        );
        assert_vec3_approx(
            m.project_point3(Vec3::new(-2.0, -1.0, -11.0)),
            Vec3::new(-1.0, -1.0, 1.0),
        );
    }

    #[test]
    fn orthographic_depth_is_linear_between_near_and_far() {
        let m = orthographic(-1.0, 1.0, -1.0, 1.0, 1.0, 11.0);
        // Halfway between near (z=-1) and far (z=-11) is z=-6.
        assert!(approx_eq(
            m.project_point3(Vec3::new(0.0, 0.0, -6.0)).z,
            0.5
        ));
    }
}

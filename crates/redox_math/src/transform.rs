use crate::{Mat4, Quat, Vec3};

/// World-space transform composed from the local [`Transform`] and ancestors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalTransform {
    /// Local-to-world matrix.
    pub matrix: Mat4,
}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }
}

impl GlobalTransform {
    /// The identity world transform.
    pub const IDENTITY: Self = Self {
        matrix: Mat4::IDENTITY,
    };

    /// Wraps an existing matrix.
    pub fn from_matrix(matrix: Mat4) -> Self {
        Self { matrix }
    }

    /// Translation part of the world matrix.
    pub fn translation(&self) -> Vec3 {
        self.matrix.to_scale_rotation_translation().2
    }
}

/// Единый компонент трансформации для всего движка
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Position relative to the parent (or the world when there is none).
    pub translation: Vec3,
    /// Orientation relative to the parent.
    pub rotation: Quat,
    /// Per-axis scale relative to the parent.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    /// Identity transform (origin, no rotation, unit scale).
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Creates a transform with only a translation.
    pub fn from_translation(t: Vec3) -> Self {
        Self {
            translation: t,
            ..Self::IDENTITY
        }
    }

    /// Creates a transform from translation and rotation.
    pub fn from_translation_rotation(t: Vec3, r: Quat) -> Self {
        Self {
            translation: t,
            rotation: r,
            ..Self::IDENTITY
        }
    }

    /// Вычисляет матрицу модели (Model Matrix) на основе параметров
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        Quat::from_axis_angle(Vec3::new(1.0, 1.0, 0.0).normalize(), 0.7)
    }

    #[test]
    fn default_transform_is_identity() {
        assert_eq!(Transform::default(), Transform::IDENTITY);
        assert_mat4_approx(Transform::default().matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn default_global_transform_is_identity_with_zero_translation() {
        assert_eq!(GlobalTransform::default(), GlobalTransform::IDENTITY);
        assert_vec3_approx(GlobalTransform::default().translation(), Vec3::ZERO);
    }

    #[test]
    fn from_translation_sets_only_the_translation() {
        let t = Vec3::new(1.0, -2.0, 3.0);
        let transform = Transform::from_translation(t);
        assert_eq!(transform.translation, t);
        assert_eq!(transform.rotation, Quat::IDENTITY);
        assert_eq!(transform.scale, Vec3::ONE);
    }

    #[test]
    fn from_translation_rotation_sets_translation_and_rotation_but_keeps_unit_scale() {
        let t = Vec3::new(4.0, 5.0, 6.0);
        let r = Quat::from_axis_angle(Vec3::Y, 0.3);
        let transform = Transform::from_translation_rotation(t, r);
        assert_eq!(transform.translation, t);
        assert_eq!(transform.rotation, r);
        assert_eq!(transform.scale, Vec3::ONE);
    }

    #[test]
    fn matrix_maps_origin_to_translation() {
        let transform = Transform {
            translation: Vec3::new(7.0, 8.0, 9.0),
            rotation: sample_rotation(),
            scale: Vec3::new(2.0, 3.0, 4.0),
        };
        assert_vec3_approx(
            transform.matrix().transform_point3(Vec3::ZERO),
            transform.translation,
        );
    }

    #[test]
    fn matrix_applies_scale_then_rotation_then_translation() {
        // (1,0,0) -> scale x2 -> (2,0,0) -> rotate 90 about Z -> (0,2,0) -> translate -> (1,2,0)
        let transform = Transform {
            translation: Vec3::new(1.0, 0.0, 0.0),
            rotation: Quat::from_axis_angle(Vec3::Z, FRAC_PI_2),
            scale: Vec3::splat(2.0),
        };
        let p = transform.matrix().transform_point3(Vec3::X);
        assert_vec3_approx(p, Vec3::new(1.0, 2.0, 0.0));
    }

    #[test]
    fn matrix_agrees_with_transform_matrix_free_function() {
        let transform = Transform {
            translation: Vec3::new(-1.0, 0.5, 2.0),
            rotation: sample_rotation(),
            scale: Vec3::new(1.0, 2.0, 0.5),
        };
        let expected =
            crate::transform_matrix(transform.translation, transform.rotation, transform.scale);
        assert_mat4_approx(transform.matrix(), expected);
    }

    #[test]
    fn matrix_decomposes_back_to_the_original_fields() {
        let transform = Transform {
            translation: Vec3::new(1.0, -2.0, 3.0),
            rotation: sample_rotation(),
            scale: Vec3::new(1.0, 2.0, 3.0),
        };
        let (scale, rotation, translation) = transform.matrix().to_scale_rotation_translation();

        assert_vec3_approx(scale, transform.scale);
        assert_vec3_approx(translation, transform.translation);
        assert!(approx_eq(rotation.dot(transform.rotation).abs(), 1.0));
    }

    #[test]
    fn zero_scale_transform_collapses_points_onto_its_translation() {
        let transform = Transform {
            translation: Vec3::new(3.0, 3.0, 3.0),
            rotation: sample_rotation(),
            scale: Vec3::ZERO,
        };
        let m = transform.matrix();
        assert!(approx_eq(m.determinant(), 0.0));
        assert_vec3_approx(
            m.transform_point3(Vec3::new(9.0, -4.0, 2.0)),
            transform.translation,
        );
    }

    #[test]
    fn child_transform_is_applied_before_parent_in_composed_global_transform() {
        // Parent: moved to (0,5,0) and rotated 90 about Z. Child: moved 1 along local X.
        // The child's origin is (1,0,0) in parent space -> (0,1,0) after the parent's rotation
        // -> (0,6,0) in world space.
        let parent = Transform {
            translation: Vec3::new(0.0, 5.0, 0.0),
            rotation: Quat::from_axis_angle(Vec3::Z, FRAC_PI_2),
            scale: Vec3::ONE,
        };
        let child = Transform::from_translation(Vec3::new(1.0, 0.0, 0.0));

        let world = GlobalTransform::from_matrix(parent.matrix() * child.matrix());
        assert_vec3_approx(world.translation(), Vec3::new(0.0, 6.0, 0.0));
    }

    #[test]
    fn reversing_parent_and_child_gives_a_different_world_position() {
        let parent = Transform {
            translation: Vec3::new(0.0, 5.0, 0.0),
            rotation: Quat::from_axis_angle(Vec3::Z, FRAC_PI_2),
            scale: Vec3::ONE,
        };
        let child = Transform::from_translation(Vec3::new(1.0, 0.0, 0.0));

        let wrong = GlobalTransform::from_matrix(child.matrix() * parent.matrix());
        assert_vec3_approx(wrong.translation(), Vec3::new(1.0, 5.0, 0.0));
    }

    #[test]
    fn global_transform_translation_ignores_rotation_and_scale() {
        let t = Vec3::new(3.0, -4.0, 5.0);
        let transform = Transform {
            translation: t,
            rotation: sample_rotation(),
            scale: Vec3::new(2.0, 3.0, 4.0),
        };
        let global = GlobalTransform::from_matrix(transform.matrix());
        assert_vec3_approx(global.translation(), t);
    }

    #[test]
    fn global_transform_from_matrix_keeps_the_matrix_unchanged() {
        let m = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let global = GlobalTransform::from_matrix(m);
        assert_eq!(global.matrix, m);
        assert_eq!(global, GlobalTransform { matrix: m });
    }
}

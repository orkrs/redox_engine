use redox_math::{Aabb, Frustum, Quat, Sphere, Vec3, look_at, perspective, transform_matrix};

#[test]
fn test_aabb_transformations() {
    let aabb = Aabb::from_center_size(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
    let rotation = Quat::from_rotation_y(45_f32.to_radians());
    let scale = Vec3::splat(2.0);
    let matrix = transform_matrix(Vec3::ZERO, rotation, scale);

    let transformed_aabb = aabb.transform(matrix);

    assert!(
        transformed_aabb.size().x * transformed_aabb.size().y * transformed_aabb.size().z > 8.0
    );

    let corner = Vec3::new(1.0, 1.0, 1.0);
    let transformed_corner = matrix.transform_point3(corner);
    assert!(transformed_aabb.contains_point(transformed_corner));
}

#[test]
fn test_frustum_extraction_and_culling() {
    let proj = perspective(90_f32.to_radians(), 1.0, 0.1, 100.0);
    let view = look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
    let frustum = Frustum::from_view_projection(proj * view);

    let center_aabb = Aabb::from_center_size(Vec3::ZERO, Vec3::splat(1.0));
    assert!(frustum.intersects_aabb(&center_aabb));

    let behind_aabb = Aabb::from_center_size(Vec3::new(0.0, 0.0, 10.0), Vec3::splat(1.0));
    assert!(!frustum.intersects_aabb(&behind_aabb));
}

#[test]
fn test_sphere_logic() {
    let sphere = Sphere::new(Vec3::new(1.0, 2.0, 3.0), 5.0);
    let other_sphere = Sphere::new(Vec3::new(2.0, 2.0, 3.0), 1.0);
    assert!(sphere.intersects(&other_sphere));
}

#[test]
fn test_coordinate_system_sanity() {
    assert!((Vec3::Y.y - 1.0).abs() < f32::EPSILON);
    let x = Vec3::X;
    let y = Vec3::Y;
    let z = x.cross(y);
    assert!((z.z - 1.0).abs() < f32::EPSILON);
}
// ---------------------------------------------------------------------------
// Phase 0.3 additions: frustum-sphere intersection, Arvo AABB transform
// ---------------------------------------------------------------------------

#[cfg(test)]
mod phase03_tests {
    use redox_math::*;

    fn camera_frustum() -> Frustum {
        let proj = perspective(90.0_f32.to_radians(), 16.0 / 9.0, 0.1, 100.0);
        let view = look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        Frustum::from_view_projection(proj * view)
    }

    #[test]
    fn frustum_sphere_inside_intersecting_outside() {
        let f = camera_frustum();

        // Center of view → definitely inside.
        assert!(f.intersects_sphere(&Sphere {
            center: Vec3::ZERO,
            radius: 0.1,
        }));

        // Grazing the near plane region → intersects.
        assert!(f.intersects_sphere(&Sphere {
            center: Vec3::new(50.0, 0.0, 5.0),
            radius: 60.0,
        }));

        // Far behind the camera → outside.
        assert!(!f.intersects_sphere(&Sphere {
            center: Vec3::new(0.0, 0.0, 20.0),
            radius: 0.5,
        }));
    }

    #[test]
    fn arvo_transform_matches_corner_method() {
        // Rotation + translation + scale; compare against the naive
        // 8-corner expansion (Arvo must match for any rigid+scale matrix).
        let rot = Quat::from_axis_angle(Vec3::Y, 37.0_f32.to_radians());
        let m = Mat4::from_rotation_translation(rot, Vec3::new(10.0, -3.0, 2.0))
            * Mat4::from_scale(Vec3::new(2.0, 1.0, 0.5));

        let aabb = Aabb::from_center_size(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 0.5, 2.0));
        let arvo = aabb.transform(m);

        // Naive reference.
        let c = aabb.center();
        let h = aabb.half_extents();
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    let t = m.transform_point3(c + Vec3::new(sx * h.x, sy * h.y, sz * h.z));
                    min = min.min(t);
                    max = max.max(t);
                }
            }
        }

        const EPS: f32 = 1e-4;
        assert!(
            (arvo.min.x - min.x).abs() < EPS,
            "min.x {} vs {}",
            arvo.min.x,
            min.x
        );
        assert!((arvo.min.y - min.y).abs() < EPS);
        assert!((arvo.min.z - min.z).abs() < EPS);
        assert!(
            (arvo.max.x - max.x).abs() < EPS,
            "max.x {} vs {}",
            arvo.max.x,
            max.x
        );
        assert!((arvo.max.y - max.y).abs() < EPS);
        assert!((arvo.max.z - max.z).abs() < EPS);
    }
}

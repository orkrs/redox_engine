//! Bounds, frustum and projection helpers.

use redox_math::bounds::{Aabb, Sphere};
use redox_math::frustum::Frustum;
use redox_math::{Mat4, Quat, Transform, Vec3, look_at, perspective};

#[test]
fn aabb_center_and_extents() {
    let b = Aabb::from_center_size(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.5, 1.0, 1.5));
    assert_eq!(b.center(), Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(b.half_extents(), Vec3::new(0.5, 1.0, 1.5));
    assert_eq!(b.size(), Vec3::new(1.0, 2.0, 3.0));
    assert!(b.contains_point(Vec3::new(1.0, 2.0, 3.0)));
    assert!(!b.contains_point(Vec3::new(5.0, 2.0, 3.0)));
}

#[test]
fn aabb_expand_grows_to_include_a_point() {
    let b = Aabb::from_center_size(Vec3::ZERO, Vec3::splat(1.0)).expand(Vec3::new(4.0, 0.0, 0.0));
    assert!(b.contains_point(Vec3::new(4.0, 0.0, 0.0)));
    assert!(b.contains_point(Vec3::new(-1.0, 0.0, 0.0)));
}

#[test]
fn aabb_transform_follows_translation() {
    let b = Aabb::from_center_size(Vec3::ZERO, Vec3::splat(1.0));
    let moved = b.transform(Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)));
    assert!((moved.center() - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-5);
}

#[test]
fn spheres_intersect_when_closer_than_their_radii() {
    let a = Sphere::new(Vec3::ZERO, 1.0);
    assert!(a.intersects(&Sphere::new(Vec3::new(1.5, 0.0, 0.0), 1.0)));
    assert!(!a.intersects(&Sphere::new(Vec3::new(3.0, 0.0, 0.0), 1.0)));
}

#[test]
fn frustum_culls_boxes_behind_the_camera() {
    let view = look_at(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
    let proj = perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
    let frustum = Frustum::from_view_projection(proj * view);

    let ahead = Aabb::from_center_size(Vec3::new(0.0, 0.0, -10.0), Vec3::splat(1.0));
    let behind = Aabb::from_center_size(Vec3::new(0.0, 0.0, 10.0), Vec3::splat(1.0));
    let far_away = Aabb::from_center_size(Vec3::new(0.0, 0.0, -500.0), Vec3::splat(1.0));
    assert!(frustum.intersects_aabb(&ahead));
    assert!(!frustum.intersects_aabb(&behind));
    assert!(!frustum.intersects_aabb(&far_away));
}

#[test]
fn perspective_maps_near_and_far_to_zero_and_one() {
    let proj = perspective(1.0, 1.0, 0.5, 50.0);
    let near = proj * Vec3::new(0.0, 0.0, -0.5).extend(1.0);
    let far = proj * Vec3::new(0.0, 0.0, -50.0).extend(1.0);
    assert!((near.z / near.w).abs() < 1e-5, "near plane -> depth 0");
    assert!((far.z / far.w - 1.0).abs() < 1e-4, "far plane -> depth 1");
}

#[test]
fn transform_defaults_to_identity() {
    let t = Transform::default();
    assert_eq!(t.translation, Vec3::ZERO);
    assert_eq!(t.rotation, Quat::IDENTITY);
    assert_eq!(t.scale, Vec3::ONE);
}

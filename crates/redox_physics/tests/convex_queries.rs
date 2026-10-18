//! Specification (tests first) for convex hull shapes in physics queries (plan item B2, last part).
//!
//! `QueryShape` is `Copy` and used everywhere, so it is NOT changed. A hull is a separate type and
//! gets its own query methods. New file `crates/redox_physics/src/convex.rs` (add `pub mod convex;` and
//! re-export `ConvexHull` from `lib.rs`):
//!
//! ```text
//! /// A convex shape built from points, for sweeps and overlap tests. Cheap to clone (shared).
//! #[derive(Clone, Debug)]
//! pub struct ConvexHull { /* SharedShape */ }
//! impl ConvexHull {
//!     /// The convex hull of `points`. `None` when there are fewer than 4 points, any coordinate
//!     /// is not finite, or the points are (nearly) coplanar so the hull has no volume.
//!     pub fn from_points(points: &[Vec3]) -> Option<ConvexHull>;
//! }
//! impl PhysicsContext {
//!     /// As `cast_shape`, with a hull instead of a `QueryShape`.
//!     pub fn cast_convex(&self, hull: &ConvexHull, position: Vec3, rotation: Quat, direction: Vec3,
//!         max_distance: f32, exclude: &[Entity], layers: u32) -> Option<ShapeHit>;
//!     /// As `overlap_shape`, with a hull.
//!     pub fn overlap_convex(&self, hull: &ConvexHull, position: Vec3, rotation: Quat,
//!         exclude: &[Entity], layers: u32) -> Vec<Entity>;
//! }
//! ```
//!
//! Implementation hint: `SharedShape::convex_hull(&points)` returns `Option<SharedShape>`; check for a
//! volume with an extent test of your own (every axis of the hull AABB must exceed 1e-4).
//! In `shapes.rs` the private helpers `cast_skipping` and `overlapping_entities` take a `QueryShape` and
//! call `to_shared()`; change them to take a `&SharedShape` (callers pass `shape.to_shared()`), so the
//! hull methods reuse them. All existing tests must pass unchanged.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Quat, Vec3};
use redox_physics::{ConvexHull, PhysicsContext, QueryShape};

const ALL: u32 = u32::MAX;

/// Eight corners of a cube with half-size `h`.
fn cube(h: f32) -> Vec<Vec3> {
    let mut p = Vec::new();
    for x in [-h, h] {
        for y in [-h, h] {
            for z in [-h, h] {
                p.push(Vec3::new(x, y, z));
            }
        }
    }
    p
}

/// A wall 1 m thick with its front face at z = -4.5, and a small box at (3, 0, 0).
fn world() -> (PhysicsContext, Entity, Entity) {
    let mut w = World::new();
    let wall = w.spawn();
    let boxed = w.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    ctx.add_rigid_body(
        wall,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, 0.0, -5.0)),
        vec![ColliderBuilder::cuboid(5.0, 5.0, 0.5)],
    )
    .unwrap();
    ctx.add_rigid_body(
        boxed,
        RigidBodyBuilder::fixed().translation(Vec3::new(3.0, 0.0, 0.0)),
        vec![ColliderBuilder::cuboid(0.5, 0.5, 0.5)],
    )
    .unwrap();
    ctx.step(1.0 / 60.0);
    (ctx, wall, boxed)
}

#[test]
fn a_hull_needs_a_volume_and_finite_points() {
    assert!(ConvexHull::from_points(&cube(0.5)).is_some());
    assert!(ConvexHull::from_points(&[]).is_none());
    assert!(
        ConvexHull::from_points(&cube(0.5)[..3]).is_none(),
        "three points"
    );
    let flat = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
    ];
    assert!(ConvexHull::from_points(&flat).is_none(), "coplanar");
    let mut bad = cube(0.5);
    bad[2] = Vec3::new(f32::NAN, 0.0, 0.0);
    assert!(ConvexHull::from_points(&bad).is_none(), "NaN");
    let tetra = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
    assert!(ConvexHull::from_points(&tetra).is_some());
}

#[test]
fn a_cube_hull_casts_like_a_box() {
    let (ctx, wall, _) = world();
    let hull = ConvexHull::from_points(&cube(0.5)).unwrap();
    let hit = ctx
        .cast_convex(
            &hull,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::NEG_Z,
            100.0,
            &[],
            ALL,
        )
        .unwrap();
    let reference = ctx
        .cast_shape(
            QueryShape::Cuboid {
                half_extents: Vec3::splat(0.5),
            },
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::NEG_Z,
            100.0,
            &[],
            ALL,
        )
        .unwrap();
    assert_eq!(hit.entity, Some(wall));
    assert!(
        (hit.distance - reference.distance).abs() < 1e-3,
        "{hit:?} vs {reference:?}"
    );
    assert!(
        (hit.distance - 4.0).abs() < 1e-3,
        "front face at -4.5, box half 0.5"
    );
}

#[test]
fn a_cast_that_misses_or_is_too_short_returns_none() {
    let (ctx, _, _) = world();
    let hull = ConvexHull::from_points(&cube(0.5)).unwrap();
    let cast = |dir: Vec3, max: f32| {
        ctx.cast_convex(&hull, Vec3::ZERO, Quat::IDENTITY, dir, max, &[], ALL)
    };
    assert!(cast(Vec3::Y, 100.0).is_none());
    assert!(cast(Vec3::NEG_Z, 2.0).is_none());
    assert!(cast(Vec3::ZERO, 100.0).is_none());
}

#[test]
fn exclusion_works_for_hull_casts() {
    let (ctx, wall, _) = world();
    let hull = ConvexHull::from_points(&cube(0.5)).unwrap();
    assert!(
        ctx.cast_convex(
            &hull,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::NEG_Z,
            100.0,
            &[wall],
            ALL
        )
        .is_none()
    );
}

#[test]
fn overlap_finds_the_entities_the_hull_touches() {
    let (ctx, _, boxed) = world();
    let hull = ConvexHull::from_points(&cube(0.5)).unwrap();
    let at = |x: f32, exclude: &[Entity]| {
        ctx.overlap_convex(&hull, Vec3::new(x, 0.0, 0.0), Quat::IDENTITY, exclude, ALL)
    };
    assert_eq!(at(2.8, &[]), vec![boxed]);
    assert!(at(10.0, &[]).is_empty());
    assert!(at(2.8, &[boxed]).is_empty());
}

#[test]
fn rotation_turns_the_hull() {
    let (ctx, _, boxed) = world();
    // A long thin plank along X (3 m half-length) reaches the box at x = 3 only when it lies along X.
    let mut points = Vec::new();
    for x in [-3.0, 3.0] {
        for y in [-0.1, 0.1] {
            for z in [-0.1, 0.1] {
                points.push(Vec3::new(x, y, z));
            }
        }
    }
    let plank = ConvexHull::from_points(&points).unwrap();
    let along_x = ctx.overlap_convex(&plank, Vec3::ZERO, Quat::IDENTITY, &[], ALL);
    assert_eq!(along_x, vec![boxed]);
    let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    assert!(
        ctx.overlap_convex(&plank, Vec3::ZERO, turned, &[], ALL)
            .is_empty()
    );
}

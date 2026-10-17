//! Scene queries and body bookkeeping of the physics context.

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::World;
use redox_math::{Quat, Vec3};
use redox_physics::{PhysicsContext, RaycastRequest};

fn world_with_wall() -> (PhysicsContext, redox_ecs::Entity) {
    let mut world = World::new();
    let wall = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    ctx.add_rigid_body(
        wall,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, 0.0, -10.0)),
        vec![ColliderBuilder::cuboid(5.0, 5.0, 0.5)],
    )
    .unwrap();
    ctx.step(1.0 / 60.0); // builds the broad phase used by queries
    (ctx, wall)
}

fn request(id: u64, direction: Vec3) -> RaycastRequest {
    RaycastRequest {
        origin: Vec3::ZERO,
        direction,
        max_distance: 50.0,
        request_id: id,
        callback_entity: None,
    }
}

#[test]
fn ray_hits_the_wall_with_a_front_facing_normal() {
    let (ctx, wall) = world_with_wall();
    let (point, normal, entity) = ctx
        .cast_ray_with_normal(Vec3::ZERO, Vec3::NEG_Z, 100.0, &[])
        .expect("the wall is in front of the ray");
    assert_eq!(entity, Some(wall));
    assert!(
        (point.z + 9.5).abs() < 1e-3,
        "front face of the wall, got {point:?}"
    );
    assert!(
        normal.z > 0.9,
        "normal faces the ray origin, got {normal:?}"
    );
}

#[test]
fn ray_misses_when_out_of_range_or_pointing_away() {
    let (ctx, _) = world_with_wall();
    assert!(
        ctx.cast_ray_with_normal(Vec3::ZERO, Vec3::NEG_Z, 5.0, &[])
            .is_none()
    );
    assert!(
        ctx.cast_ray_with_normal(Vec3::ZERO, Vec3::Z, 100.0, &[])
            .is_none()
    );
    assert!(
        ctx.cast_ray_with_normal(Vec3::ZERO, Vec3::ZERO, 100.0, &[])
            .is_none()
    );
}

#[test]
fn excluded_entities_are_ignored() {
    let (ctx, wall) = world_with_wall();
    assert!(
        ctx.cast_ray_with_normal(Vec3::ZERO, Vec3::NEG_Z, 100.0, &[wall])
            .is_none()
    );
    let hit = ctx.cast_ray_excluding(Vec3::ZERO, Vec3::new(0.0, 0.0, -20.0), &[]);
    assert_eq!(hit.map(|(_, e)| e), Some(wall));
    assert!(
        ctx.cast_ray_excluding(Vec3::ZERO, Vec3::new(0.0, 0.0, -20.0), &[wall])
            .is_none()
    );
}

#[test]
fn queued_raycasts_are_answered_in_order() {
    let (mut ctx, wall) = world_with_wall();
    ctx.push_raycast_request(request(7, Vec3::NEG_Z));
    ctx.push_raycast_request(request(8, Vec3::Z));
    let results = ctx.drain_raycast_results();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].request_id, 7);
    assert!(results[0].hit);
    assert_eq!(results[0].entity, Some(wall));
    assert_eq!(results[1].request_id, 8);
    assert!(!results[1].hit);
    assert!(
        ctx.drain_raycast_results().is_empty(),
        "the queue is drained"
    );
}

#[test]
fn bodies_can_be_moved_read_and_removed() {
    let mut world = World::new();
    let e = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    let (handle, _) = ctx
        .add_rigid_body(
            e,
            RigidBodyBuilder::dynamic(),
            vec![ColliderBuilder::ball(0.5)],
        )
        .unwrap();
    assert_eq!(ctx.get_body_handle(e), Some(handle));

    ctx.set_rigid_body_transform(handle, Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY);
    let (pos, _) = ctx.get_rigid_body_transform(handle).unwrap();
    assert_eq!(pos, Vec3::new(1.0, 2.0, 3.0));

    ctx.remove_rigid_body(e).unwrap();
    assert!(ctx.get_body_handle(e).is_none());
    assert!(
        ctx.remove_rigid_body(e).is_err(),
        "second removal reports the missing entity"
    );
}

#[test]
fn gravity_pulls_dynamic_bodies_down() {
    let mut world = World::new();
    let e = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -10.0, 0.0));
    let (handle, _) = ctx
        .add_rigid_body(
            e,
            RigidBodyBuilder::dynamic(),
            vec![ColliderBuilder::ball(0.5)],
        )
        .unwrap();
    for _ in 0..60 {
        ctx.step(1.0 / 60.0);
    }
    let (pos, _) = ctx.get_rigid_body_transform(handle).unwrap();
    assert!(pos.y < -4.0, "fell for one second, y = {}", pos.y);
}

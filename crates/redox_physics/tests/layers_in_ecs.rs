//! Specification (tests first) for ECS-driven collision layers (plan item B1, second part).
//!
//! Behaviour to implement in `redox_physics`: `CollisionLayers` (already exists in `shapes.rs`) becomes
//! usable as an ECS component. `sync_to_physics(world)` (in `sync.rs`, also called by `tick_physics`)
//! applies the component of every simulated entity to that entity's colliders through
//! `PhysicsContext::set_collider_layers`, and does so again whenever the component value changed since
//! the last sync. Entities without the component keep their layers. No new public API is required
//! (`CollisionLayers` must be `Clone`; it already is `Copy`). Keep it cheap: remember the last applied
//! value per entity inside `PhysicsContext` (e.g. a `HashMap<Entity, CollisionLayers>` field) and call
//! `set_collider_layers` only for entities whose value differs. Forget the entry when the body is removed.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::{Quat, Vec3};
use redox_physics::{CollisionLayers, PhysicsContext, QueryShape, sync::sync_to_physics};

const ALL: u32 = u32::MAX;
const PROBE: QueryShape = QueryShape::Sphere { radius: 100.0 };

fn world_with_body() -> (World, Entity) {
    let mut world = World::new();
    let e = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    ctx.add_rigid_body(
        e,
        RigidBodyBuilder::fixed(),
        vec![ColliderBuilder::ball(1.0)],
    )
    .unwrap();
    ctx.step(1.0 / 60.0);
    world.insert_resource(ctx);
    (world, e)
}

fn found(world: &World, mask: u32) -> bool {
    world.get_resource::<PhysicsContext>().unwrap().overlap_any(
        PROBE,
        Vec3::ZERO,
        Quat::IDENTITY,
        &[],
        mask,
    )
}

#[test]
fn the_component_is_applied_by_the_sync() {
    let (mut world, e) = world_with_body();
    assert!(
        found(&world, 0b0100),
        "default layers: member of everything"
    );
    world.add_component(e, CollisionLayers::new(0b0010, ALL));
    sync_to_physics(&mut world);
    assert!(found(&world, 0b0010), "now a member of layer 2");
    assert!(!found(&world, 0b0100), "no longer a member of layer 3");
}

#[test]
fn changing_the_component_changes_the_colliders_again() {
    let (mut world, e) = world_with_body();
    world.add_component(e, CollisionLayers::new(0b0010, ALL));
    sync_to_physics(&mut world);
    *world.get_component_mut::<CollisionLayers>(e).unwrap() = CollisionLayers::new(0b1000, ALL);
    sync_to_physics(&mut world);
    assert!(found(&world, 0b1000));
    assert!(!found(&world, 0b0010));
}

#[test]
fn entities_without_the_component_keep_their_layers() {
    let (mut world, e) = world_with_body();
    world
        .get_resource_mut::<PhysicsContext>()
        .unwrap()
        .set_collider_layers(e, CollisionLayers::new(0b0001, ALL))
        .unwrap();
    sync_to_physics(&mut world);
    sync_to_physics(&mut world);
    assert!(found(&world, 0b0001));
    assert!(!found(&world, 0b0010));
}

#[test]
fn a_manual_change_is_not_overwritten_while_the_component_is_unchanged() {
    // The component says layer 2 and was applied once. A later direct call wins until the
    // component itself changes (the sync only reacts to component changes).
    let (mut world, e) = world_with_body();
    world.add_component(e, CollisionLayers::new(0b0010, ALL));
    sync_to_physics(&mut world);
    world
        .get_resource_mut::<PhysicsContext>()
        .unwrap()
        .set_collider_layers(e, CollisionLayers::new(0b0001, ALL))
        .unwrap();
    sync_to_physics(&mut world);
    assert!(found(&world, 0b0001));
    assert!(!found(&world, 0b0010));
}

#[test]
fn removing_the_body_forgets_the_entity() {
    let (mut world, e) = world_with_body();
    world.add_component(e, CollisionLayers::new(0b0010, ALL));
    sync_to_physics(&mut world);
    world
        .get_resource_mut::<PhysicsContext>()
        .unwrap()
        .remove_rigid_body(e)
        .unwrap();
    sync_to_physics(&mut world); // must not panic
    assert!(!found(&world, ALL));
}

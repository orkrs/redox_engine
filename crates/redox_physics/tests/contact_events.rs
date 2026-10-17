//! Specification (tests first) for contact events and physics materials.
//!
//! API to implement in `redox_physics` (re-exported from the crate root; put the types in a new
//! module `contacts.rs`):
//!
//! ```text
//! pub struct ContactEnter { pub a: Entity, pub b: Entity, pub point: Vec3, pub normal: Vec3, pub impulse: f32 }
//! pub struct ContactExit  { pub a: Entity, pub b: Entity }
//! pub enum   ContactEvent { Enter(ContactEnter), Exit(ContactExit) }
//! pub struct PhysicsMaterial { pub friction: f32, pub restitution: f32 }   // Default: 0.5 / 0.0
//! impl PhysicsContext {
//!     /// Opt in or out of contact events for the colliders of `entity`. Events are only produced for
//!     /// pairs where at least one entity opted in. Default: off.
//!     pub fn set_contact_events(&mut self, entity: Entity, enabled: bool) -> Result<(), PhysicsError>;
//!     /// Takes all contact events collected since the last call (oldest first).
//!     pub fn drain_contact_events(&mut self) -> Vec<ContactEvent>;
//!     pub fn set_physics_material(&mut self, entity: Entity, material: PhysicsMaterial) -> Result<(), PhysicsError>;
//! }
//! ```
//!
//! Notes: `a` and `b` are the two entities of the pair (order unspecified); `normal` is a unit vector
//! along the contact normal; `point` is a world-space contact point; `impulse` is the total normal
//! impulse of the contact in the step in which it began (> 0 for a real impact). Also send the events
//! into the ECS from `tick_physics` as `Events<ContactEvent>` when that resource exists (follow how
//! `TriggerEnter` is sent in `sync.rs`).

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_ecs::{Entity, World};
use redox_math::Vec3;
use redox_physics::{ContactEvent, PhysicsContext, PhysicsMaterial};

const DT: f32 = 1.0 / 60.0;

struct Drop {
    ctx: PhysicsContext,
    floor: Entity,
    ball: Entity,
    handle: rapier3d::dynamics::RigidBodyHandle,
}

/// A floor (top at y = 0) and a ball of radius 0.5 whose centre starts at `start_y`.
fn dropped_ball(start_y: f32) -> Drop {
    let mut world = World::new();
    let floor = world.spawn();
    let ball = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(50.0, 0.5, 50.0)],
    )
    .unwrap();
    let (handle, _) = ctx
        .add_rigid_body(
            ball,
            RigidBodyBuilder::dynamic().translation(Vec3::new(0.0, start_y, 0.0)),
            vec![ColliderBuilder::ball(0.5)],
        )
        .unwrap();
    Drop {
        ctx,
        floor,
        ball,
        handle,
    }
}

fn pair_is(a: Entity, b: Entity, x: Entity, y: Entity) -> bool {
    (a == x && b == y) || (a == y && b == x)
}

#[test]
fn a_landing_ball_reports_one_enter_with_a_vertical_normal_and_a_real_impulse() {
    let mut scene = dropped_ball(2.5);
    scene.ctx.set_contact_events(scene.ball, true).unwrap();
    for _ in 0..120 {
        scene.ctx.step(DT);
    }
    let events = scene.ctx.drain_contact_events();
    let enters: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ContactEvent::Enter(enter) => Some(*enter),
            ContactEvent::Exit(_) => None,
        })
        .collect();
    assert_eq!(enters.len(), 1, "a ball that comes to rest lands once: {events:?}");
    let enter = enters[0];
    assert!(pair_is(enter.a, enter.b, scene.floor, scene.ball));
    assert!((enter.normal.length() - 1.0).abs() < 1e-3, "unit normal: {:?}", enter.normal);
    assert!(enter.normal.y.abs() > 0.9, "vertical normal: {:?}", enter.normal);
    assert!(enter.impulse > 0.5, "falling 2 m gives a real impact: {}", enter.impulse);
    assert!(enter.point.y.abs() < 0.2, "contact on the floor surface: {:?}", enter.point);
    assert!(scene.ctx.drain_contact_events().is_empty(), "drain empties the queue");
}

#[test]
fn events_are_opt_in() {
    let mut scene = dropped_ball(2.5);
    for _ in 0..120 {
        scene.ctx.step(DT);
    }
    assert!(scene.ctx.drain_contact_events().is_empty());
}

#[test]
fn opting_in_on_either_entity_of_the_pair_is_enough() {
    let mut scene = dropped_ball(2.5);
    scene.ctx.set_contact_events(scene.floor, true).unwrap();
    for _ in 0..120 {
        scene.ctx.step(DT);
    }
    assert!(!scene.ctx.drain_contact_events().is_empty());
}

#[test]
fn a_bouncing_ball_alternates_enter_and_exit() {
    let mut scene = dropped_ball(2.5);
    for entity in [scene.floor, scene.ball] {
        scene
            .ctx
            .set_physics_material(entity, PhysicsMaterial { friction: 0.5, restitution: 0.9 })
            .unwrap();
    }
    scene.ctx.set_contact_events(scene.ball, true).unwrap();
    for _ in 0..360 {
        scene.ctx.step(DT);
    }
    let events = scene.ctx.drain_contact_events();
    assert!(events.len() >= 4, "at least two bounces: {events:?}");
    assert!(matches!(events[0], ContactEvent::Enter(_)));
    assert!(matches!(events[1], ContactEvent::Exit(_)));
    assert!(matches!(events[2], ContactEvent::Enter(_)));
    // Strict alternation: never two Enters or two Exits in a row.
    for pair in events.windows(2) {
        let same = matches!(
            (&pair[0], &pair[1]),
            (ContactEvent::Enter(_), ContactEvent::Enter(_))
                | (ContactEvent::Exit(_), ContactEvent::Exit(_))
        );
        assert!(!same, "events alternate: {pair:?}");
    }
}

/// Highest the ball gets after it first touches the floor.
fn peak_after_first_contact(restitution: f32) -> f32 {
    let mut scene = dropped_ball(2.0);
    for entity in [scene.floor, scene.ball] {
        scene
            .ctx
            .set_physics_material(entity, PhysicsMaterial { friction: 0.5, restitution })
            .unwrap();
    }
    let mut touched = false;
    let mut peak = 0.0f32;
    for _ in 0..240 {
        scene.ctx.step(DT);
        let y = scene.ctx.get_rigid_body_transform(scene.handle).unwrap().0.y;
        if touched {
            peak = peak.max(y);
        } else if y < 0.58 {
            touched = true;
        }
    }
    peak
}

#[test]
fn restitution_decides_how_high_the_ball_bounces() {
    assert!(peak_after_first_contact(0.9) > 1.0, "bouncy ball rebounds high");
    assert!(peak_after_first_contact(0.0) < 0.7, "dead ball stays down");
}

/// Distance a box slides over the floor in one second when launched at 5 m/s.
fn slide_distance(friction: f32) -> f32 {
    let mut world = World::new();
    let floor = world.spawn();
    let block = world.spawn();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(100.0, 0.5, 100.0)],
    )
    .unwrap();
    let (handle, _) = ctx
        .add_rigid_body(
            block,
            RigidBodyBuilder::dynamic()
                .translation(Vec3::new(0.0, 0.5, 0.0))
                .linvel(Vec3::new(5.0, 0.0, 0.0)),
            vec![ColliderBuilder::cuboid(0.5, 0.5, 0.5)],
        )
        .unwrap();
    for entity in [floor, block] {
        ctx.set_physics_material(entity, PhysicsMaterial { friction, restitution: 0.0 })
            .unwrap();
    }
    for _ in 0..60 {
        ctx.step(DT);
    }
    ctx.get_rigid_body_transform(handle).unwrap().0.x
}

#[test]
fn friction_decides_how_far_a_box_slides() {
    assert!(slide_distance(0.0) > 4.5, "frictionless: keeps its speed");
    assert!(slide_distance(1.0) < 2.0, "high friction: stops early");
}

#[test]
fn materials_and_events_need_a_known_entity() {
    let mut scene = dropped_ball(2.5);
    let mut other = World::new();
    // Entity ids restart per World: spawn past the two used by the scene.
    let stranger = (0..5).map(|_| other.spawn()).last().unwrap();
    assert!(scene.ctx.set_contact_events(stranger, true).is_err());
    assert!(scene.ctx.set_physics_material(stranger, PhysicsMaterial::default()).is_err());
}

#[test]
fn the_default_material_is_ordinary() {
    let m = PhysicsMaterial::default();
    assert_eq!((m.friction, m.restitution), (0.5, 0.0));
}

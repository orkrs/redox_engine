//! Kinematic character controller wrapping Rapier's `KinematicCharacterController`.

use rapier3d::control::{
    CharacterAutostep, CharacterLength, KinematicCharacterController as RapierKcc,
};
use rapier3d::prelude::*;
use redox_ecs::World;
use redox_math::{Transform, Vec3};

use crate::components::{CharacterController, CharacterSettings, Collider, RigidBody};
use crate::context::PhysicsContext;

/// Builds Rapier's controller from the character's settings. While the character rises
/// (`vertical > 0`) snapping to the ground is off, so a jump is not pulled back down at once.
fn build_kcc(settings: &CharacterSettings, vertical: f32) -> RapierKcc {
    let autostep = (settings.step_height > 0.0).then_some(CharacterAutostep {
        max_height: CharacterLength::Absolute(settings.step_height),
        min_width: CharacterLength::Absolute(settings.step_min_width),
        include_dynamic_bodies: false,
    });
    let snap_to_ground = (vertical <= 0.0 && settings.snap_to_ground > 0.0)
        .then_some(CharacterLength::Absolute(settings.snap_to_ground));
    RapierKcc {
        max_slope_climb_angle: settings.max_slope_angle,
        autostep,
        snap_to_ground,
        ..RapierKcc::default()
    }
}

/// Translation the ground under a character moved by since the last step; zero when it stood on
/// nothing or its collider is gone.
///
/// Call order in `tick_physics`: `move_character_controllers` runs before the Rapier step, so the
/// collider positions read here are those of the previous step. The carry is the difference of two
/// consecutive positions, so the error does not accumulate; it lags by one frame.
fn platform_carry(colliders: &ColliderSet, ground_pose: Option<(ColliderHandle, Vec3)>) -> Vec3 {
    let Some((handle, previous)) = ground_pose else {
        return Vec3::ZERO;
    };
    colliders.get(handle).map_or(Vec3::ZERO, |collider| {
        collider.position().translation - previous
    })
}

/// Moves every [`CharacterController`] this step: gravity, wall slide, autostep, moving platforms.
pub fn move_character_controllers(world: &mut World, dt: f32) {
    let Some(physics) = world.remove_resource::<PhysicsContext>() else {
        return;
    };
    let entities: Vec<_> = world
        .query::<(
            redox_ecs::Entity,
            &CharacterController,
            &Transform,
            &RigidBody,
            &Collider,
        )>()
        .iter(world)
        .map(|(e, c, t, rb, col)| {
            (
                e,
                c.desired_translation,
                c.gravity,
                c.vertical_velocity,
                c.settings,
                c.ground_pose,
                *t,
                rb.handle,
                col.handle,
            )
        })
        .collect();

    let mut results = Vec::with_capacity(entities.len());
    let no_bodies = RigidBodySet::new();
    for (
        entity,
        desired,
        gravity,
        vertical,
        settings,
        ground_pose,
        transform,
        body_handle,
        collider_handle,
    ) in entities
    {
        let Some(collider) = physics.colliders.get(collider_handle) else {
            continue;
        };
        let shape = collider.shape();
        // Semi-implicit Euler: accelerate first, then move with the new speed.
        let mut vy = vertical - gravity * dt;
        let mut wish = desired + platform_carry(&physics.colliders, ground_pose);
        wish.y += vy * dt;

        let iso = Pose::from_parts(transform.translation, transform.rotation);
        let filter = QueryFilter::default().exclude_rigid_body(body_handle);
        let kcc = build_kcc(&settings, vertical);
        // Rapier's controller replaces part of the motion of a character touching a *kinematic* body by that body's
        // velocity. For position-driven platforms this froze the character (it could not walk on them at all), so the
        // query sees no bodies and treats every collider as plain geometry; `platform_carry` moves the character instead.
        let queries = physics.broad_phase.as_query_pipeline(
            physics.narrow_phase.query_dispatcher(),
            &no_bodies,
            &physics.colliders,
            filter,
        );
        // The last upward-facing hit within the slope limit is the floor the character stands on.
        let mut floor = None;
        let movement = kcc.move_shape(dt, &queries, shape, &iso, wish, |collision| {
            if kcc.up.angle_between(collision.hit.normal1) <= kcc.max_slope_climb_angle {
                floor = Some(collision.handle);
            }
        });
        let delta = movement.translation;
        let grounded = movement.grounded;
        let ground_collider = if grounded { floor } else { None };
        let next_ground_pose = ground_collider.and_then(|handle| {
            physics
                .colliders
                .get(handle)
                .map(|collider| (handle, collider.position().translation))
        });
        // Landing stops the fall; hitting a ceiling stops the rise.
        let hit_ceiling = vy > 0.0 && delta.y < wish.y - 1e-4;
        if (grounded && vy < 0.0) || hit_ceiling {
            vy = 0.0;
        }
        results.push((
            entity,
            delta,
            grounded,
            vy,
            body_handle,
            ground_collider,
            next_ground_pose,
        ));
    }

    world.insert_resource(physics);

    for (entity, delta, grounded, vy, body_handle, ground_collider, ground_pose) in results {
        if let Some(t) = world.get_component_mut::<Transform>(entity) {
            t.translation += delta;
        }
        if let Some(cc) = world.get_component_mut::<CharacterController>(entity) {
            cc.grounded = grounded;
            cc.desired_translation = Vec3::ZERO;
            cc.vertical_velocity = vy;
            cc.ground_collider = ground_collider;
            cc.ground_pose = ground_pose;
        }
        let translation = world
            .get_component::<Transform>(entity)
            .map(|t| (t.translation, t.rotation));
        if let (Some((pos, rot)), Some(physics)) =
            (translation, world.get_resource_mut::<PhysicsContext>())
        {
            physics.set_rigid_body_transform(body_handle, pos, rot);
        }
    }
}

#[cfg(test)]
mod kcc_tests {
    use rapier3d::prelude::*;
    use redox_ecs::World;
    use redox_math::{Transform, Vec3};

    use crate::components::{CharacterController, Collider, Kinematic, RigidBody};
    use crate::context::PhysicsContext;
    use crate::sync::tick_physics;

    fn spawn_body(
        world: &mut World,
        translation: Vec3,
        body: RigidBodyBuilder,
        collider: ColliderBuilder,
    ) -> redox_ecs::Entity {
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(translation));
        let mut phys = world.remove_resource::<PhysicsContext>().unwrap();
        let (bh, cols) = phys
            .add_rigid_body(e, body.translation(translation), vec![collider])
            .unwrap();
        world.insert_resource(phys);
        world.add_component(e, RigidBody { handle: bh });
        world.add_component(e, Collider { handle: cols[0] });
        e
    }

    #[test]
    fn character_lands_on_floor() {
        let mut world = World::new();
        world.insert_resource(PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0)));
        spawn_body(
            &mut world,
            Vec3::new(0.0, -0.5, 0.0),
            RigidBodyBuilder::fixed(),
            ColliderBuilder::cuboid(10.0, 0.5, 10.0),
        );
        let player = spawn_body(
            &mut world,
            Vec3::new(0.0, 2.0, 0.0),
            RigidBodyBuilder::kinematic_position_based(),
            ColliderBuilder::capsule_y(0.4, 0.3),
        );
        world.add_component(player, Kinematic);
        world.add_component(
            player,
            CharacterController {
                desired_translation: Vec3::ZERO,
                gravity: 20.0,
                grounded: false,
                ..Default::default()
            },
        );

        for _ in 0..40 {
            tick_physics(&mut world, 1.0 / 60.0);
        }
        let y = world
            .get_component::<Transform>(player)
            .unwrap()
            .translation
            .y;
        let grounded = world
            .get_component::<CharacterController>(player)
            .unwrap()
            .grounded;
        assert!(y > 0.2 && y < 1.6, "landed y={y}");
        assert!(grounded, "expected grounded after falling onto the floor");
    }

    #[test]
    fn character_does_not_walk_through_wall() {
        let mut world = World::new();
        world.insert_resource(PhysicsContext::new(Vec3::ZERO));
        spawn_body(
            &mut world,
            Vec3::new(0.0, -0.5, 0.0),
            RigidBodyBuilder::fixed(),
            ColliderBuilder::cuboid(10.0, 0.5, 10.0),
        );
        spawn_body(
            &mut world,
            Vec3::new(2.0, 0.5, 0.0),
            RigidBodyBuilder::fixed(),
            ColliderBuilder::cuboid(0.25, 1.0, 2.0),
        );
        let player = spawn_body(
            &mut world,
            Vec3::new(0.0, 0.9, 0.0),
            RigidBodyBuilder::kinematic_position_based(),
            ColliderBuilder::capsule_y(0.4, 0.3),
        );
        world.add_component(player, Kinematic);
        world.add_component(player, CharacterController::default());

        for _ in 0..30 {
            world
                .get_component_mut::<CharacterController>(player)
                .unwrap()
                .desired_translation = Vec3::new(8.0 / 60.0, 0.0, 0.0);
            tick_physics(&mut world, 1.0 / 60.0);
        }
        let x = world
            .get_component::<Transform>(player)
            .unwrap()
            .translation
            .x;
        assert!(x < 1.6, "walked through wall, x={x}");
    }

    #[test]
    fn trigger_enter_is_sent() {
        use crate::components::Trigger;
        use crate::trigger::TriggerEnter;
        use redox_ecs::Events;

        let mut world = World::new();
        world.insert_resource(PhysicsContext::new(Vec3::ZERO));
        world.insert_resource(Events::<TriggerEnter>::new());

        let crystal = world.spawn();
        let mut phys = world.remove_resource::<PhysicsContext>().unwrap();
        let (bh, col) = phys
            .add_trigger(crystal, Vec3::ZERO, ColliderBuilder::ball(0.5))
            .unwrap();
        world.insert_resource(phys);
        world.add_component(crystal, RigidBody { handle: bh });
        world.add_component(crystal, Collider { handle: col });
        world.add_component(crystal, Trigger);

        let player = spawn_body(
            &mut world,
            Vec3::new(0.1, 0.0, 0.0),
            RigidBodyBuilder::dynamic(),
            ColliderBuilder::ball(0.4),
        );
        let _ = player;
        tick_physics(&mut world, 1.0 / 60.0);
        let events = world.get_resource::<Events<TriggerEnter>>().unwrap();
        assert!(
            events.iter().next().is_some(),
            "expected TriggerEnter when overlapping a sensor"
        );
    }
}

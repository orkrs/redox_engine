//! Kinematic character controller wrapping Rapier's `KinematicCharacterController`.

use rapier3d::control::{
    CharacterAutostep, CharacterLength, KinematicCharacterController as RapierKcc,
};
use rapier3d::prelude::*;
use redox_ecs::World;
use redox_math::{Quat, Transform, Vec3};

use crate::components::{CharacterController, CharacterSettings, Collider, RigidBody};
use crate::context::PhysicsContext;

/// Margin added to the slope drop of a step, so rounding does not leave the character floating.
const SNAP_MARGIN: f32 = 0.01;

/// Snap distance for a step of `step_len` horizontal length: the configured snap, lengthened to the
/// drop one step can cause on a slope of `max_slope_angle`, and capped at `max_snap_to_ground`.
fn snap_distance(settings: &CharacterSettings, step_len: f32) -> f32 {
    let slope_drop = step_len * settings.max_slope_angle.tan() + SNAP_MARGIN;
    let limit = settings.max_snap_to_ground.max(settings.snap_to_ground);
    settings.snap_to_ground.max(slope_drop).min(limit)
}

/// Builds Rapier's controller from the character's settings. While the character rises
/// (`vertical > 0`) snapping to the ground is off, so a jump is not pulled back down at once.
fn build_kcc(settings: &CharacterSettings, vertical: f32, step_len: f32) -> RapierKcc {
    let autostep = (settings.step_height > 0.0).then_some(CharacterAutostep {
        max_height: CharacterLength::Absolute(settings.step_height),
        min_width: CharacterLength::Absolute(settings.step_min_width),
        include_dynamic_bodies: false,
    });
    let snap_to_ground = (vertical <= 0.0 && settings.snap_to_ground > 0.0)
        .then(|| CharacterLength::Absolute(snap_distance(settings, step_len)));
    RapierKcc {
        max_slope_climb_angle: settings.max_slope_angle,
        autostep,
        snap_to_ground,
        ..RapierKcc::default()
    }
}

/// Turns `point` by `rotation` about `pivot`.
fn rotate_about(point: Vec3, pivot: Vec3, rotation: Quat) -> Vec3 {
    pivot + rotation * (point - pivot)
}

/// Heading of `rotation` about the vertical axis in radians: the angle of its forward (`+Z`) vector
/// measured from `+Z` towards `+X`. For a pure yaw rotation this is the turn angle.
fn yaw_of(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::Z;
    forward.x.atan2(forward.z)
}

/// Current pose of the body that owns the collider `handle`.
///
/// Reads the rigid body, not the collider: `sync_to_physics` writes the platform's new transform
/// into the body right before the step, but the collider only takes that pose inside
/// `PhysicsPipeline::step`. Reading the collider here gave the character the platform one tick
/// stale (PHY-06: the carry lagged the platform by a tick). Colliders without a parent body fall
/// back to their own pose.
fn platform_pose(
    colliders: &ColliderSet,
    bodies: &RigidBodySet,
    handle: ColliderHandle,
) -> Option<(Vec3, Quat)> {
    let collider = colliders.get(handle)?;
    let pose = collider
        .parent()
        .and_then(|body| bodies.get(body))
        .map_or(*collider.position(), |body| *body.position());
    Some((pose.translation, pose.rotation))
}

/// Motion of the ground under a character since the last step, at the point `at` where it stands:
/// the translation of that point (including the platform's rotation about its origin) and the
/// rotation the platform turned by. Zero motion when the character stood on nothing or its collider
/// is gone.
///
/// `ground` holds the platform pose recorded at the previous step; the current pose comes from
/// `platform_pose`. The carry is the difference of two consecutive poses, so the error does not
/// accumulate.
fn platform_carry(
    colliders: &ColliderSet,
    bodies: &RigidBodySet,
    ground: Option<(ColliderHandle, Vec3, Quat)>,
    at: Vec3,
) -> (Vec3, Quat) {
    let Some((handle, previous, previous_rotation)) = ground else {
        return (Vec3::ZERO, Quat::IDENTITY);
    };
    platform_pose(colliders, bodies, handle).map_or(
        (Vec3::ZERO, Quat::IDENTITY),
        |(position, rotation)| {
            let turn = rotation * previous_rotation.inverse();
            // The point turned with the platform about its previous origin, then moved with the origin.
            let translation = rotate_about(at, previous, turn) + (position - previous) - at;
            (translation, turn)
        },
    )
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
        // Entities switched off by `ScriptInactive` stay where they are.
        .filter(|(e, ..)| world.get_component::<redox_ecs::Inactive>(*e).is_none())
        .map(|(e, c, t, rb, col)| {
            (
                e,
                c.desired_translation,
                c.gravity,
                c.vertical_velocity,
                c.settings,
                c.ground_pose
                    .map(|(h, p)| (h, p, c.ground_rotation.unwrap_or(Quat::IDENTITY))),
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
        let (carry, platform_turn) = platform_carry(
            &physics.colliders,
            &physics.rigid_bodies,
            ground_pose,
            transform.translation,
        );
        let mut wish = desired + carry;
        wish.y += vy * dt;
        let yaw = if settings.inherit_platform_yaw {
            yaw_of(platform_turn)
        } else {
            0.0
        };

        let iso = Pose::from_parts(transform.translation, transform.rotation);
        let filter = QueryFilter::default().exclude_rigid_body(body_handle);
        let step_len = (wish.x * wish.x + wish.z * wish.z).sqrt();
        let kcc = build_kcc(&settings, vertical, step_len);
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
            platform_pose(&physics.colliders, &physics.rigid_bodies, handle)
                .map(|(position, rotation)| (handle, position, rotation))
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
            yaw,
        ));
    }

    world.insert_resource(physics);

    for (entity, delta, grounded, vy, body_handle, ground_collider, ground_pose, yaw) in results {
        if let Some(t) = world.get_component_mut::<Transform>(entity) {
            t.translation += delta;
            if yaw != 0.0 {
                t.rotation = Quat::from_rotation_y(yaw) * t.rotation;
            }
        }
        if let Some(cc) = world.get_component_mut::<CharacterController>(entity) {
            cc.grounded = grounded;
            cc.desired_translation = Vec3::ZERO;
            cc.vertical_velocity = vy;
            cc.ground_collider = ground_collider;
            cc.ground_pose = ground_pose.map(|(handle, position, _)| (handle, position));
            cc.ground_rotation = ground_pose.map(|(_, _, rotation)| rotation);
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

#[cfg(test)]
mod carry_math_tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn rotate_about_turns_a_point_a_quarter_turn_around_the_pivot() {
        let turned = rotate_about(
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        );
        // Offset (1, 0, 0) from the pivot turns to (0, 0, -1) under a +90 degree turn about Y.
        assert!(
            (turned - Vec3::new(2.0, 0.0, -1.0)).length() < EPS,
            "{turned:?}"
        );
    }

    #[test]
    fn rotate_about_leaves_the_pivot_fixed() {
        let pivot = Vec3::new(-4.0, 1.0, 2.5);
        let turned = rotate_about(pivot, pivot, Quat::from_rotation_y(1.2));
        assert!((turned - pivot).length() < EPS);
    }

    #[test]
    fn yaw_of_a_pure_yaw_rotation_is_its_angle() {
        for angle in [-2.0_f32, -0.3, 0.0, 0.3, 1.0, 3.0] {
            let measured = yaw_of(Quat::from_rotation_y(angle));
            assert!(
                (measured - angle).abs() < EPS,
                "angle {angle}: got {measured}"
            );
        }
    }

    #[test]
    fn yaw_of_identity_is_zero() {
        assert!(yaw_of(Quat::IDENTITY).abs() < EPS);
    }

    #[test]
    fn platform_carry_without_ground_is_zero() {
        let colliders = ColliderSet::new();
        let bodies = RigidBodySet::new();
        let (translation, turn) = platform_carry(&colliders, &bodies, None, Vec3::ONE);
        assert_eq!(translation, Vec3::ZERO);
        assert_eq!(turn, Quat::IDENTITY);
    }
}

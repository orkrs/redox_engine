//! Activating a ragdoll from rest must not throw the figure up (PHY-07 activation jump).

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_anim::Skeleton;
use redox_core::ragdoll_system::{RagdollRig, enter_ragdoll};
use redox_ecs::{Entity, World};
use redox_math::{Mat4, Vec3};
use redox_physics::{CharacterController, PhysicsContext, RagdollTemplate};

const DT: f32 = 1.0 / 60.0;
const SECONDS: f32 = 2.0;
/// The figure is lifted this far above the floor so that no body starts inside it.
const LIFT: f32 = 0.05;
/// Largest rise of the centre of mass allowed after activation, in metres.
const MAX_RISE: f32 = 0.1;

const NAMES: [&str; 15] = [
    "mixamorig:Hips",
    "mixamorig:Spine",
    "mixamorig:Head",
    "mixamorig:LeftArm",
    "mixamorig:LeftForeArm",
    "mixamorig:LeftHand",
    "mixamorig:RightArm",
    "mixamorig:RightForeArm",
    "mixamorig:RightHand",
    "mixamorig:LeftUpLeg",
    "mixamorig:LeftLeg",
    "mixamorig:LeftFoot",
    "mixamorig:RightUpLeg",
    "mixamorig:RightLeg",
    "mixamorig:RightFoot",
];
const PARENTS: [Option<usize>; 15] = [
    None,
    Some(0),
    Some(1),
    Some(1),
    Some(3),
    Some(4),
    Some(1),
    Some(6),
    Some(7),
    Some(0),
    Some(9),
    Some(10),
    Some(0),
    Some(12),
    Some(13),
];
const LOCAL: [[f32; 3]; 15] = [
    [0.0, 1.0, 0.0],
    [0.0, 0.2, 0.0],
    [0.0, 0.5, 0.0],
    [0.2, 0.45, 0.0],
    [0.0, -0.3, 0.0],
    [0.0, -0.25, 0.0],
    [-0.2, 0.45, 0.0],
    [0.0, -0.3, 0.0],
    [0.0, -0.25, 0.0],
    [0.1, -0.1, 0.0],
    [0.0, -0.45, 0.0],
    [0.0, -0.4, 0.0],
    [-0.1, -0.1, 0.0],
    [0.0, -0.45, 0.0],
    [0.0, -0.4, 0.0],
];

fn skeleton() -> Skeleton {
    Skeleton::from_local_bind(
        (0..15)
            .map(|i| {
                let [x, y, z] = LOCAL[i];
                (
                    NAMES[i].to_string(),
                    PARENTS[i],
                    Mat4::from_translation(Vec3::new(x, y, z)),
                )
            })
            .collect(),
    )
}

/// Mean position of the ragdoll bodies: the figure's centre of mass, taking every body as equal.
fn centre_of_mass(physics: &PhysicsContext, rig: &RagdollRig) -> f32 {
    let heights: Vec<f32> = rig
        .ragdoll
        .bodies()
        .iter()
        .filter_map(|entity| physics.get_body_handle(*entity))
        .filter_map(|handle| physics.get_rigid_body_transform(handle))
        .map(|(translation, _)| translation.y)
        .collect();
    heights.iter().sum::<f32>() / heights.len() as f32
}

#[test]
fn a_ragdoll_activated_from_rest_does_not_rise_by_more_than_ten_centimetres() {
    let skeleton = skeleton();
    let template = RagdollTemplate::humanoid(&skeleton).unwrap();
    let mut world = World::new();
    let mut physics = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));

    let floor = world.spawn();
    physics
        .add_rigid_body(
            floor,
            RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
            vec![ColliderBuilder::cuboid(20.0, 0.5, 20.0)],
        )
        .unwrap();

    // The character's own capsule encloses the standing figure, as a real controller collider does.
    let character: Entity = world.spawn();
    physics
        .add_rigid_body(
            character,
            RigidBodyBuilder::kinematic_position_based().translation(Vec3::new(0.0, 0.6, 0.0)),
            vec![ColliderBuilder::capsule_y(0.5, 0.3)],
        )
        .unwrap();
    world.add_component(character, CharacterController::default());

    let model: Vec<Mat4> = skeleton
        .global_matrices(&skeleton.bind_local_pose())
        .into_iter()
        .map(|m| Mat4::from_translation(Vec3::Y * LIFT) * m)
        .collect();
    enter_ragdoll(
        &mut physics,
        &mut world,
        character,
        &skeleton,
        &template,
        &model,
        Vec3::ZERO,
        2,
    )
    .unwrap();
    let rig = world
        .get_component::<RagdollRig>(character)
        .unwrap()
        .clone();

    let start = centre_of_mass(&physics, &rig);
    let mut highest = start;
    let steps = (SECONDS / DT) as usize;
    for _ in 0..steps {
        physics.step(DT);
        highest = highest.max(centre_of_mass(&physics, &rig));
    }
    assert!(
        highest - start <= MAX_RISE,
        "centre of mass rose {} m after activation (start {start}, highest {highest})",
        highest - start
    );
}

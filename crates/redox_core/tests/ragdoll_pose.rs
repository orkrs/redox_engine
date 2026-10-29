//! Ragdoll mode of a character: pose hand-over between animation and bodies, without a GPU.

#![allow(clippy::unwrap_used)]

use redox_anim::Skeleton;
use redox_core::ragdoll_system::{
    CharacterPoseMode, RagdollRig, enter_ragdoll, exit_ragdoll, ragdoll_pose_system,
};
use redox_ecs::{Entity, World};
use redox_math::{Mat4, Vec3};
use redox_physics::{CharacterController, PhysicsContext, RagdollTemplate, bone_world_from_body};

const DT: f32 = 1.0 / 60.0;

/// Bone names, parents and local translations of the standard 15-bone humanoid.
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

/// A character entity with a controller, in a world holding the physics context.
fn character_world(gravity: Vec3) -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(PhysicsContext::new(gravity));
    let entity = world.spawn();
    world.add_component(entity, CharacterController::default());
    (world, entity)
}

/// Animated model-space matrices of the skeleton, lifted by `lift` and turned by `yaw`.
fn animated_model(skeleton: &Skeleton, lift: f32, yaw: f32) -> Vec<Mat4> {
    let turn = Mat4::from_translation(Vec3::Y * lift) * Mat4::from_rotation_y(yaw);
    skeleton
        .global_matrices(&skeleton.bind_local_pose())
        .into_iter()
        .map(|m| turn * m)
        .collect()
}

/// Enters the ragdoll mode at `model`, keeping the physics context in the world between calls.
fn enter(world: &mut World, entity: Entity, skeleton: &Skeleton, model: &[Mat4]) {
    let template = RagdollTemplate::humanoid(skeleton).unwrap();
    let mut physics = world.remove_resource::<PhysicsContext>().unwrap();
    enter_ragdoll(
        &mut physics,
        world,
        entity,
        skeleton,
        &template,
        model,
        Vec3::ZERO,
        2,
    )
    .unwrap();
    world.insert_resource(physics);
}

fn rig(world: &World, entity: Entity) -> RagdollRig {
    world.get_component::<RagdollRig>(entity).unwrap().clone()
}

/// World matrix of the body of template bone `index`.
fn body_matrix(world: &World, rig: &RagdollRig, index: usize) -> Mat4 {
    let physics = world.get_resource::<PhysicsContext>().unwrap();
    let body = rig.ragdoll.bodies()[index];
    let handle = physics.get_body_handle(body).unwrap();
    let (t, r) = physics.get_rigid_body_transform(handle).unwrap();
    Mat4::from_rotation_translation(r, t)
}

fn max_abs_diff(a: &Mat4, b: &Mat4) -> f32 {
    a.to_cols_array()
        .iter()
        .zip(b.to_cols_array())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn bone_poses_follow_the_bodies() {
    let skeleton = skeleton();
    let model = animated_model(&skeleton, 2.0, 0.0);
    let (mut world, entity) = character_world(Vec3::new(0.0, -9.81, 0.0));
    enter(&mut world, entity, &skeleton, &model);
    for _ in 0..30 {
        world.get_resource_mut::<PhysicsContext>().unwrap().step(DT);
    }
    ragdoll_pose_system(&mut world);
    let rig = rig(&world, entity);
    let template = RagdollTemplate::humanoid(&skeleton).unwrap();
    for (index, bone) in template.bones.iter().enumerate() {
        let expected = bone_world_from_body(
            body_matrix(&world, &rig, index),
            Vec3::from_array(bone.offset),
        );
        assert!(
            max_abs_diff(&rig.bone_world[index], &expected) < 1e-4,
            "bone {} does not follow its body",
            bone.bone
        );
    }
}

#[test]
fn switching_to_ragdoll_keeps_the_pose_in_the_first_frame() {
    let skeleton = skeleton();
    let model = animated_model(&skeleton, 1.0, 0.3);
    let (mut world, entity) = character_world(Vec3::ZERO);
    enter(&mut world, entity, &skeleton, &model);
    ragdoll_pose_system(&mut world);
    let skin = rig(&world, entity).skin_matrices();
    for (bone, (matrix, inverse)) in model.iter().zip(&skeleton.bones).enumerate() {
        let animated = *matrix * inverse.inv_bind;
        assert!(
            max_abs_diff(&skin[bone], &animated) < 1e-3,
            "bone {bone} jumps on entering the ragdoll"
        );
    }
}

#[test]
fn switching_back_restores_the_animator_output() {
    let skeleton = skeleton();
    let model = animated_model(&skeleton, 1.0, 0.0);
    let (mut world, entity) = character_world(Vec3::ZERO);
    enter(&mut world, entity, &skeleton, &model);
    let mut physics = world.remove_resource::<PhysicsContext>().unwrap();
    exit_ragdoll(&mut physics, &mut world, entity);
    world.insert_resource(physics);
    assert!(world.get_component::<RagdollRig>(entity).is_none());
    assert_eq!(
        world.get_component::<CharacterPoseMode>(entity),
        Some(&CharacterPoseMode::Animated)
    );
    assert!(world.get_component::<CharacterController>(entity).is_some());
    let physics = world.get_resource::<PhysicsContext>().unwrap();
    assert_eq!(physics.rigid_bodies.len(), 0, "the bodies are gone");
}

#[test]
fn the_controller_is_inactive_while_ragdolled() {
    let skeleton = skeleton();
    let model = animated_model(&skeleton, 1.0, 0.0);
    let (mut world, entity) = character_world(Vec3::ZERO);
    enter(&mut world, entity, &skeleton, &model);
    assert!(world.get_component::<CharacterController>(entity).is_none());
    assert_eq!(
        world.get_component::<CharacterPoseMode>(entity),
        Some(&CharacterPoseMode::Ragdoll)
    );
}

#[test]
fn a_skinned_mesh_receives_the_ragdoll_pose() {
    // The head is template bone 2 and skeleton bone 2. Moving its body by one metre upwards moves
    // the head's skinning transform of the bind-pose head origin by the same amount.
    let skeleton = skeleton();
    let model = animated_model(&skeleton, 1.0, 0.0);
    let (mut world, entity) = character_world(Vec3::ZERO);
    enter(&mut world, entity, &skeleton, &model);
    ragdoll_pose_system(&mut world);
    // The bind-space head origin (a vertex at the head joint), and where the animated head is now.
    let bind = skeleton.global_matrices(&skeleton.bind_local_pose());
    let bind_head = bind[2].transform_point3(Vec3::ZERO);
    let animated_head = model[2].transform_point3(Vec3::ZERO);
    let before = rig(&world, entity).skin_matrices()[2].transform_point3(bind_head);
    assert!(
        (before - animated_head).length() < 1e-3,
        "starts at the animated head"
    );

    let rig = rig(&world, entity);
    let head = rig.ragdoll.bodies()[2];
    let mut physics = world.remove_resource::<PhysicsContext>().unwrap();
    let handle = physics.get_body_handle(head).unwrap();
    let (translation, rotation) = physics.get_rigid_body_transform(handle).unwrap();
    physics.set_rigid_body_transform(handle, translation + Vec3::Y, rotation);
    world.insert_resource(physics);
    ragdoll_pose_system(&mut world);

    let after = rig(&world, entity).skin_matrices()[2].transform_point3(bind_head);
    assert!(
        ((after - before) - Vec3::Y).length() < 1e-3,
        "the head moved by {:?}, expected +Y",
        after - before
    );
}

//! Ragdoll templates (humanoid layout, RON, validation) and spawned ragdolls in the physics world.

#![allow(clippy::unwrap_used)]

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder, RigidBodyHandle};
use redox_anim::Skeleton;
use redox_ecs::{Entity, World};
use redox_math::{Mat4, Vec3};
use redox_physics::{
    PhysicsContext, RagdollError, RagdollHandle, RagdollJointKind, RagdollTemplate,
};

const DT: f32 = 1.0 / 60.0;
const DEG: f32 = std::f32::consts::PI / 180.0;

/// Bone names of the standard 15-bone Mixamo-style humanoid, in skeleton order.
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

/// Parent of each bone of the humanoid skeleton.
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

/// Local translation of each bone relative to its parent (a standing rest pose).
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

/// Builds the humanoid skeleton with the given bone names (a name the layout does not know
/// makes that bone missing).
fn skeleton_named(names: &[&str; 15]) -> Skeleton {
    Skeleton::from_local_bind(
        (0..15)
            .map(|i| {
                let [x, y, z] = LOCAL[i];
                (
                    names[i].to_string(),
                    PARENTS[i],
                    Mat4::from_translation(Vec3::new(x, y, z)),
                )
            })
            .collect(),
    )
}

fn humanoid_skeleton() -> Skeleton {
    skeleton_named(&NAMES)
}

/// World poses of the template's bones, lifted by `lift` metres along Y.
fn lifted_poses(template: &RagdollTemplate, skeleton: &Skeleton, lift: f32) -> Vec<Mat4> {
    let model = skeleton.global_matrices(&skeleton.bind_local_pose());
    template
        .bone_poses(skeleton, &model)
        .unwrap()
        .into_iter()
        .map(|m| Mat4::from_translation(Vec3::Y * lift) * m)
        .collect()
}

/// A static floor whose top is at y = 0.
fn add_floor(ctx: &mut PhysicsContext, world: &mut World) {
    let floor = world.spawn();
    ctx.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(20.0, 0.5, 20.0)],
    )
    .unwrap();
}

fn spawn(
    ctx: &mut PhysicsContext,
    world: &mut World,
    skeleton: &Skeleton,
    lift: f32,
    velocity: Vec3,
    group: u32,
) -> RagdollHandle {
    let template = RagdollTemplate::humanoid(skeleton).unwrap();
    let poses = lifted_poses(&template, skeleton, lift);
    ctx.spawn_ragdoll(world, &template, &poses, velocity, group)
        .unwrap()
}

fn body_handle(ctx: &PhysicsContext, entity: Entity) -> RigidBodyHandle {
    ctx.get_body_handle(entity).unwrap()
}

/// World matrix of a body.
fn body_matrix(ctx: &PhysicsContext, entity: Entity) -> Mat4 {
    let (translation, rotation) = ctx
        .get_rigid_body_transform(body_handle(ctx, entity))
        .unwrap();
    Mat4::from_rotation_translation(rotation, translation)
}

/// Largest linear speed of any body of the ragdoll.
fn max_speed(ctx: &PhysicsContext, ragdoll: &RagdollHandle) -> f32 {
    ragdoll
        .bodies()
        .iter()
        .map(|e| ctx.rigid_bodies[body_handle(ctx, *e)].linvel().length())
        .fold(0.0, f32::max)
}

fn run(ctx: &mut PhysicsContext, seconds: f32) {
    for _ in 0..(seconds / DT).round() as usize {
        ctx.step(DT);
    }
}

// ---- template: layout, validation, RON ----

#[test]
fn the_humanoid_template_covers_a_standard_skeleton() {
    let skeleton = humanoid_skeleton();
    let template = RagdollTemplate::humanoid(&skeleton).unwrap();
    assert_eq!(template.bones.len(), 15);
    assert!(template.bones[0].joint.is_none(), "the root has no joint");
    assert!(template.bones[1..].iter().all(|b| b.joint.is_some()));
    template.validate(&skeleton).unwrap();
}

#[test]
fn a_missing_bone_is_reported_with_its_name() {
    // "LeftFoot" renamed to something the layout does not recognise.
    let mut names = NAMES;
    names[11] = "Unused";
    let err = RagdollTemplate::humanoid(&skeleton_named(&names)).unwrap_err();
    match err {
        RagdollError::MissingBone(name) => assert!(name.contains("foot"), "{name}"),
        other => panic!("expected MissingBone, got {other:?}"),
    }
}

#[test]
fn parents_precede_children() {
    let template = RagdollTemplate::humanoid(&humanoid_skeleton()).unwrap();
    for (index, bone) in template.bones.iter().enumerate() {
        if let Some(parent) = bone.parent {
            assert!(parent < index, "{} precedes its parent", bone.bone);
        }
    }
    template.check_structure().unwrap();
}

#[test]
fn the_template_roundtrips_through_ron() {
    let template = RagdollTemplate::humanoid(&humanoid_skeleton()).unwrap();
    let text = ron::ser::to_string_pretty(&template, ron::ser::PrettyConfig::default()).unwrap();
    let parsed = RagdollTemplate::parse_ron(&text).unwrap();
    assert_eq!(parsed, template);
}

#[test]
fn bone_names_with_side_suffixes_are_recognised() {
    let names = [
        "Hips",
        "Spine",
        "Head",
        "upperarm.L",
        "forearm.L",
        "hand.L",
        "UpperArm_Right",
        "Forearm_Right",
        "Hand_Right",
        "thigh.L",
        "calf.L",
        "foot.L",
        "RightThigh",
        "calf_r",
        "foot_r",
    ];
    let skeleton = skeleton_named(&names);
    let template = RagdollTemplate::humanoid(&skeleton).unwrap();
    assert_eq!(template.bones.len(), 15);
    template.validate(&skeleton).unwrap();
}

#[test]
fn masses_sum_to_a_plausible_body_mass() {
    let template = RagdollTemplate::humanoid(&humanoid_skeleton()).unwrap();
    let total: f32 = template.bones.iter().map(|b| b.mass).sum();
    assert!((55.0..=100.0).contains(&total), "total mass {total} kg");
}

#[test]
fn the_knee_and_elbow_are_hinges_and_the_hip_a_ball() {
    let template = RagdollTemplate::humanoid(&humanoid_skeleton()).unwrap();
    let calf = template
        .bones
        .iter()
        .find(|b| b.bone.ends_with("LeftLeg"))
        .unwrap();
    match calf.joint.as_ref().unwrap().kind {
        RagdollJointKind::Hinge { min, max, .. } => {
            assert_eq!(min, 0.0);
            assert!((max - 2.4).abs() < 1e-6);
        }
        ref other => panic!("knee should be a hinge, got {other:?}"),
    }
    let thigh = template
        .bones
        .iter()
        .find(|b| b.bone.ends_with("LeftUpLeg"))
        .unwrap();
    match thigh.joint.as_ref().unwrap().kind {
        RagdollJointKind::Ball { swing, .. } => assert!((swing - 70.0 * DEG).abs() < 1e-5),
        ref other => panic!("hip should be a ball, got {other:?}"),
    }
}

// ---- spawned ragdolls ----

#[test]
fn a_spawned_ragdoll_falls_and_lands_on_the_floor() {
    let skeleton = humanoid_skeleton();
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_floor(&mut ctx, &mut world);
    let ragdoll = spawn(&mut ctx, &mut world, &skeleton, 1.5, Vec3::ZERO, 2);
    run(&mut ctx, 3.0);
    let hips = ragdoll.bodies()[0];
    let height = body_matrix(&ctx, hips).w_axis.y;
    assert!(height <= 0.4, "hips at {height} m after 3 s");
    let speed = ctx.rigid_bodies[body_handle(&ctx, hips)].linvel().length();
    assert!(speed < 0.2, "hips still moving at {speed} m/s");
}

#[test]
fn the_joints_keep_the_body_connected() {
    let skeleton = humanoid_skeleton();
    let template = RagdollTemplate::humanoid(&skeleton).unwrap();
    let poses = lifted_poses(&template, &skeleton, 1.0);
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_floor(&mut ctx, &mut world);
    let ragdoll = ctx
        .spawn_ragdoll(&mut world, &template, &poses, Vec3::ZERO, 2)
        .unwrap();
    // Joint points in each body's frame, from the spawn pose: child bone origin.
    let spawn_bodies: Vec<Mat4> = poses
        .iter()
        .zip(&template.bones)
        .map(|(p, b)| *p * Mat4::from_translation(Vec3::from_array(b.offset)))
        .collect();
    let joints: Vec<(usize, usize, Vec3)> = template
        .bones
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let parent = b.parent?;
            let origin = poses[i].w_axis.truncate();
            let local = spawn_bodies[parent].inverse().transform_point3(origin);
            Some((parent, i, local))
        })
        .collect();
    run(&mut ctx, 2.0);
    for (parent, child, anchor_in_parent) in joints {
        let parent_point =
            body_matrix(&ctx, ragdoll.bodies()[parent]).transform_point3(anchor_in_parent);
        let offset = Vec3::from_array(template.bones[child].offset);
        let child_origin = body_matrix(&ctx, ragdoll.bodies()[child]).transform_point3(-offset);
        let gap = (parent_point - child_origin).length();
        assert!(gap <= 0.03, "joint {child} opened by {gap} m");
    }
}

#[test]
fn the_knee_does_not_bend_backwards() {
    let skeleton = humanoid_skeleton();
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_floor(&mut ctx, &mut world);
    let ragdoll = spawn(&mut ctx, &mut world, &skeleton, 1.0, Vec3::ZERO, 2);
    // Push the left calf backwards, against the knee's stop at 0 rad.
    let calf = ragdoll.bodies()[10];
    ctx.set_rigid_body_velocity(
        body_handle(&ctx, calf),
        Vec3::ZERO,
        Vec3::new(-10.0, 0.0, 0.0),
    );
    run(&mut ctx, 2.0);
    let thigh_q = ctx
        .get_rigid_body_transform(body_handle(&ctx, ragdoll.bodies()[9]))
        .unwrap()
        .1;
    let calf_q = ctx
        .get_rigid_body_transform(body_handle(&ctx, calf))
        .unwrap()
        .1;
    let (axis, angle) = (thigh_q.inverse() * calf_q).to_axis_angle();
    let bend = angle * axis.x;
    assert!(
        bend >= -3.0 * DEG,
        "knee bent backwards: {} deg",
        bend / DEG
    );
    assert!(bend <= 2.4 + 3.0 * DEG, "knee past its stop: {bend}");
}

#[test]
fn the_initial_velocity_is_inherited() {
    let skeleton = humanoid_skeleton();
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    let ragdoll = spawn(
        &mut ctx,
        &mut world,
        &skeleton,
        0.0,
        Vec3::new(5.0, 0.0, 0.0),
        2,
    );
    let start: Vec<f32> = ragdoll
        .bodies()
        .iter()
        .map(|e| body_matrix(&ctx, *e).w_axis.x)
        .collect();
    run(&mut ctx, 0.2);
    for (entity, x0) in ragdoll.bodies().iter().zip(start) {
        let moved = body_matrix(&ctx, *entity).w_axis.x - x0;
        assert!(moved >= 0.5, "body moved only {moved} m forward");
    }
}

#[test]
fn removing_a_ragdoll_removes_all_bodies_and_joints() {
    let skeleton = humanoid_skeleton();
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    let ragdoll = spawn(&mut ctx, &mut world, &skeleton, 0.0, Vec3::ZERO, 2);
    assert_eq!(ctx.rigid_bodies.len(), 15);
    assert_eq!(ctx.impulse_joint_set.len(), 14);
    ctx.remove_ragdoll(&ragdoll);
    assert_eq!(ctx.rigid_bodies.len(), 0);
    assert_eq!(ctx.impulse_joint_set.len(), 0);
}

#[test]
fn adjacent_bones_do_not_collide_with_each_other() {
    let skeleton = humanoid_skeleton();
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_floor(&mut ctx, &mut world);
    let ragdoll = spawn(&mut ctx, &mut world, &skeleton, 1.0, Vec3::ZERO, 2);
    let mut fastest = 0.0_f32;
    for _ in 0..120 {
        ctx.step(DT);
        fastest = fastest.max(max_speed(&ctx, &ragdoll));
    }
    assert!(
        fastest < 20.0,
        "a body reached {fastest} m/s: the stack exploded"
    );
}

#[test]
fn two_ragdolls_do_not_interact_through_their_own_layer_but_collide_with_the_world() {
    let skeleton = humanoid_skeleton();
    // Same group, no gravity, same place: no contact between them, so no motion.
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::ZERO);
    let first = spawn(&mut ctx, &mut world, &skeleton, 0.0, Vec3::ZERO, 2);
    let second = spawn(&mut ctx, &mut world, &skeleton, 0.0, Vec3::ZERO, 2);
    run(&mut ctx, 0.5);
    assert!(max_speed(&ctx, &first) < 1e-3);
    assert!(max_speed(&ctx, &second) < 1e-3);

    // Same group again, now on a floor under gravity: the floor stops them.
    let mut world = World::new();
    let mut ctx = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    add_floor(&mut ctx, &mut world);
    let first = spawn(&mut ctx, &mut world, &skeleton, 1.0, Vec3::ZERO, 2);
    let second = spawn(&mut ctx, &mut world, &skeleton, 1.0, Vec3::ZERO, 2);
    run(&mut ctx, 2.0);
    for ragdoll in [&first, &second] {
        let hips_y = body_matrix(&ctx, ragdoll.bodies()[0]).w_axis.y;
        assert!(hips_y > -0.02, "hips fell through the floor: {hips_y}");
    }
}

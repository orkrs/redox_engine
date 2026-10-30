//! Ragdoll demo (PHY-07, step 5), without a window.
//!
//! Builds a 15-bone humanoid skeleton, drops it as a ragdoll on a floor, steps the physics for
//! three seconds and prints the height of every body part twice a second. Exits with a failure
//! code when the figure has not come to rest on the floor (hips below 0.4 m and slow).
//!
//! `--throw` gives the figure a forward speed of 5 m/s, `--lift <metres>` sets the drop height.
//!
//! ```text
//! cargo run --example ragdoll_demo -p redox_core --release
//! ```

use std::process::ExitCode;

use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use redox_anim::Skeleton;
use redox_ecs::World;
use redox_math::{Mat4, Vec3};
use redox_physics::{PhysicsContext, RagdollTemplate};

const DT: f32 = 1.0 / 60.0;
const SECONDS: f32 = 3.0;

/// Bones of a standard Mixamo-style humanoid, in skeleton order, with their parents and the
/// translation of each bone from its parent in a standing rest pose.
const BONES: [(&str, Option<usize>, [f32; 3]); 15] = [
    ("mixamorig:Hips", None, [0.0, 1.0, 0.0]),
    ("mixamorig:Spine", Some(0), [0.0, 0.2, 0.0]),
    ("mixamorig:Head", Some(1), [0.0, 0.5, 0.0]),
    ("mixamorig:LeftArm", Some(1), [0.2, 0.45, 0.0]),
    ("mixamorig:LeftForeArm", Some(3), [0.0, -0.3, 0.0]),
    ("mixamorig:LeftHand", Some(4), [0.0, -0.25, 0.0]),
    ("mixamorig:RightArm", Some(1), [-0.2, 0.45, 0.0]),
    ("mixamorig:RightForeArm", Some(6), [0.0, -0.3, 0.0]),
    ("mixamorig:RightHand", Some(7), [0.0, -0.25, 0.0]),
    ("mixamorig:LeftUpLeg", Some(0), [0.1, -0.1, 0.0]),
    ("mixamorig:LeftLeg", Some(9), [0.0, -0.45, 0.0]),
    ("mixamorig:LeftFoot", Some(10), [0.0, -0.4, 0.0]),
    ("mixamorig:RightUpLeg", Some(0), [-0.1, -0.1, 0.0]),
    ("mixamorig:RightLeg", Some(12), [0.0, -0.45, 0.0]),
    ("mixamorig:RightFoot", Some(13), [0.0, -0.4, 0.0]),
];

fn skeleton() -> Skeleton {
    Skeleton::from_local_bind(
        BONES
            .iter()
            .map(|(name, parent, [x, y, z])| {
                (
                    (*name).to_owned(),
                    *parent,
                    Mat4::from_translation(Vec3::new(*x, *y, *z)),
                )
            })
            .collect(),
    )
}

/// Value that follows `flag` in `args`, parsed.
fn flag<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
    let at = args.iter().position(|arg| arg == name)?;
    args.get(at + 1)?.parse().ok()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lift: f32 = flag(&args, "--lift").unwrap_or(1.5);
    let velocity = if args.iter().any(|arg| arg == "--throw") {
        Vec3::new(0.0, 0.0, 5.0)
    } else {
        Vec3::ZERO
    };

    let skeleton = skeleton();
    let template = match RagdollTemplate::humanoid(&skeleton) {
        Ok(template) => template,
        Err(error) => {
            eprintln!("cannot build the ragdoll template: {error:?}");
            return ExitCode::FAILURE;
        }
    };
    let model = skeleton.global_matrices(&skeleton.bind_local_pose());
    let poses: Vec<Mat4> = match template.bone_poses(&skeleton, &model) {
        Ok(poses) => poses
            .into_iter()
            .map(|pose| Mat4::from_translation(Vec3::Y * lift) * pose)
            .collect(),
        Err(error) => {
            eprintln!("cannot pose the ragdoll: {error:?}");
            return ExitCode::FAILURE;
        }
    };

    let mut world = World::new();
    let mut physics = PhysicsContext::new(Vec3::new(0.0, -9.81, 0.0));
    let floor = world.spawn();
    if let Err(error) = physics.add_rigid_body(
        floor,
        RigidBodyBuilder::fixed().translation(Vec3::new(0.0, -0.5, 0.0)),
        vec![ColliderBuilder::cuboid(20.0, 0.5, 20.0)],
    ) {
        eprintln!("cannot add the floor: {error:?}");
        return ExitCode::FAILURE;
    }
    let ragdoll = match physics.spawn_ragdoll(&mut world, &template, &poses, velocity, 2) {
        Ok(ragdoll) => ragdoll,
        Err(error) => {
            eprintln!("cannot spawn the ragdoll: {error:?}");
            return ExitCode::FAILURE;
        }
    };

    let body_position = |physics: &PhysicsContext, index: usize| -> Option<Vec3> {
        let handle = physics.get_body_handle(ragdoll.bodies()[index])?;
        physics.get_rigid_body_transform(handle).map(|(t, _)| t)
    };

    println!("bones: {}", template.bones.len());
    let steps = (SECONDS / DT) as usize;
    let mut hips = Vec3::ZERO;
    for step in 0..=steps {
        if step % 30 == 0 {
            let heights: Vec<String> = (0..template.bones.len())
                .filter_map(|i| body_position(&physics, i))
                .map(|p| format!("{:.2}", p.y))
                .collect();
            println!("t={:.1}s  heights: {}", step as f32 * DT, heights.join(" "));
        }
        physics.step(DT);
        if let Some(p) = body_position(&physics, 0) {
            hips = p;
        }
    }
    println!("hips at ({:.2}, {:.2}, {:.2})", hips.x, hips.y, hips.z);
    if hips.y < 0.4 {
        println!("ragdoll_demo: ok, the figure lies on the floor");
        ExitCode::SUCCESS
    } else {
        eprintln!("ragdoll_demo: the hips are still {:.2} m high", hips.y);
        ExitCode::FAILURE
    }
}

//! A procedural humanoid (rigid limbs on an 11-bone skeleton), its clips and its animation graph.
//!
//! The character faces +Z with its feet on y = 0. Root motion comes from the root bone's z track
//! in the walk and run clips (1.5 m/s and 4.0 m/s).

use std::f32::consts::TAU;

use redox_anim::graph::{AnimGraph, Compare, Condition, LayerDef, Motion, StateDef, TransitionDef};
use redox_anim::{AnimationClip, Keyframe, Skeleton, SkinnedGeometry, SkinnedModel};
use redox_math::{Mat4, Quat, Vec3};

pub const ROOT: usize = 0;
pub const WALK_SPEED: f32 = 1.5;
pub const RUN_SPEED: f32 = 4.0;

/// Clip indices in [`humanoid`]'s model.
pub const IDLE: usize = 0;
pub const WALK: usize = 1;
pub const RUN: usize = 2;
pub const WAVE: usize = 3;

/// `(name, parent, global bind position)` of every bone.
const BONES: [(&str, Option<usize>, [f32; 3]); 11] = [
    ("root", None, [0.0, 0.95, 0.0]),
    ("spine", Some(0), [0.0, 1.15, 0.0]),
    ("head", Some(1), [0.0, 1.5, 0.0]),
    ("upper_arm_l", Some(1), [-0.28, 1.45, 0.0]),
    ("lower_arm_l", Some(3), [-0.28, 1.15, 0.0]),
    ("upper_arm_r", Some(1), [0.28, 1.45, 0.0]),
    ("lower_arm_r", Some(5), [0.28, 1.15, 0.0]),
    ("upper_leg_l", Some(0), [-0.12, 0.95, 0.0]),
    ("lower_leg_l", Some(7), [-0.12, 0.5, 0.0]),
    ("upper_leg_r", Some(0), [0.12, 0.95, 0.0]),
    ("lower_leg_r", Some(9), [0.12, 0.5, 0.0]),
];

/// `(bone, from, to, radius at from, radius at to)` of every body part.
const PARTS: [(usize, [f32; 3], [f32; 3], f32, f32); 11] = [
    (0, [0.0, 0.85, 0.0], [0.0, 1.05, 0.0], 0.17, 0.17),
    (1, [0.0, 1.05, 0.0], [0.0, 1.5, 0.0], 0.18, 0.2),
    (2, [0.0, 1.5, 0.0], [0.0, 1.78, 0.0], 0.12, 0.1),
    (3, [-0.28, 1.45, 0.0], [-0.28, 1.15, 0.0], 0.06, 0.05),
    (4, [-0.28, 1.15, 0.0], [-0.28, 0.85, 0.0], 0.05, 0.045),
    (5, [0.28, 1.45, 0.0], [0.28, 1.15, 0.0], 0.06, 0.05),
    (6, [0.28, 1.15, 0.0], [0.28, 0.85, 0.0], 0.05, 0.045),
    (7, [-0.12, 0.95, 0.0], [-0.12, 0.5, 0.0], 0.085, 0.07),
    (8, [-0.12, 0.5, 0.0], [-0.12, 0.05, 0.0], 0.07, 0.06),
    (9, [0.12, 0.95, 0.0], [0.12, 0.5, 0.0], 0.085, 0.07),
    (10, [0.12, 0.5, 0.0], [0.12, 0.05, 0.0], 0.07, 0.06),
];

const SIDES: usize = 10;

fn skeleton() -> Skeleton {
    Skeleton::from_local_bind(
        BONES
            .iter()
            .map(|(name, parent, global)| {
                let global = Vec3::from(*global);
                let local = match parent {
                    Some(p) => global - Vec3::from(BONES[*p].2),
                    None => global,
                };
                (name.to_string(), *parent, Mat4::from_translation(local))
            })
            .collect(),
    )
}

/// A tapered cylinder with caps, rigidly skinned to `bone`.
fn add_part(g: &mut SkinnedGeometry, bone: usize, from: Vec3, to: Vec3, r0: f32, r1: f32) {
    let axis = (to - from).normalize();
    let side = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = axis.cross(side).normalize();
    let v = axis.cross(u);
    let joint = [bone as u16, 0, 0, 0];
    let weight = [1.0, 0.0, 0.0, 0.0];
    let base = g.positions.len() as u32;
    for (center, radius) in [(from, r0), (to, r1)] {
        for s in 0..SIDES {
            let theta = TAU * s as f32 / SIDES as f32;
            let radial = u * theta.cos() + v * theta.sin();
            g.positions.push((center + radial * radius).to_array());
            g.normals.push(radial.to_array());
            g.uvs.push([s as f32 / SIDES as f32, 0.0]);
            g.joints.push(joint);
            g.weights.push(weight);
        }
    }
    for s in 0..SIDES as u32 {
        let n = (s + 1) % SIDES as u32;
        let (a, b, c, d) = (
            base + s,
            base + n,
            base + SIDES as u32 + s,
            base + SIDES as u32 + n,
        );
        g.indices.extend_from_slice(&[a, c, b, b, c, d]);
    }
    for (center, radius, normal, flip) in [(from, r0, -axis, true), (to, r1, axis, false)] {
        let hub = g.positions.len() as u32;
        g.positions.push(center.to_array());
        g.normals.push(normal.to_array());
        g.uvs.push([0.5, 0.5]);
        g.joints.push(joint);
        g.weights.push(weight);
        let ring = g.positions.len() as u32;
        for s in 0..SIDES {
            let theta = TAU * s as f32 / SIDES as f32;
            let p = center + (u * theta.cos() + v * theta.sin()) * radius;
            g.positions.push(p.to_array());
            g.normals.push(normal.to_array());
            g.uvs.push([0.5, 0.5]);
            g.joints.push(joint);
            g.weights.push(weight);
        }
        for s in 0..SIDES as u32 {
            let n = (s + 1) % SIDES as u32;
            if flip {
                g.indices.extend_from_slice(&[hub, ring + n, ring + s]);
            } else {
                g.indices.extend_from_slice(&[hub, ring + s, ring + n]);
            }
        }
    }
}

fn key(time: f32, translation: Vec3, rotation: Quat) -> Keyframe {
    Keyframe {
        time,
        translation,
        rotation,
        scale: Vec3::ONE,
    }
}

const KEYS: usize = 16;

/// A looping or one-shot clip: `pose(bone name, phase 0..1)` gives the rotation and an extra
/// translation added to the bone's bind offset.
fn make_clip(
    skeleton: &Skeleton,
    name: &str,
    duration: f32,
    only: Option<&[&str]>,
    pose: impl Fn(&str, f32) -> (Quat, Vec3),
) -> AnimationClip {
    let mut clip = AnimationClip {
        name: name.into(),
        duration,
        ..Default::default()
    };
    for bone in &skeleton.bones {
        if only.is_some_and(|names| !names.contains(&bone.name.as_str())) {
            continue;
        }
        let bind = bone.bind_pose.to_scale_rotation_translation().2;
        let keys = (0..=KEYS)
            .map(|k| {
                let phase = k as f32 / KEYS as f32;
                let (rotation, extra) = pose(&bone.name, phase);
                key(duration * phase, bind + extra, rotation)
            })
            .collect();
        clip.tracks.insert(bone.name.clone(), keys);
    }
    clip
}

fn locomotion(
    skeleton: &Skeleton,
    name: &str,
    duration: f32,
    swing: f32,
    distance: f32,
) -> AnimationClip {
    make_clip(skeleton, name, duration, None, |bone, phase| {
        let a = TAU * phase;
        let rot_x = |angle: f32| Quat::from_rotation_x(angle);
        match bone {
            "root" => (
                Quat::IDENTITY,
                Vec3::new(0.0, 0.03 * swing * (2.0 * a).sin().abs(), distance * phase),
            ),
            "spine" => (Quat::from_rotation_y(0.12 * swing * a.sin()), Vec3::ZERO),
            "upper_leg_l" => (rot_x(swing * a.sin()), Vec3::ZERO),
            "upper_leg_r" => (rot_x(-swing * a.sin()), Vec3::ZERO),
            "lower_leg_l" => (
                rot_x(-0.5 * swing * (a + 1.2).sin().max(0.0) - 0.1),
                Vec3::ZERO,
            ),
            "lower_leg_r" => (
                rot_x(-0.5 * swing * (a + 1.2 + TAU / 2.0).sin().max(0.0) - 0.1),
                Vec3::ZERO,
            ),
            "upper_arm_l" => (rot_x(-0.8 * swing * a.sin()), Vec3::ZERO),
            "upper_arm_r" => (rot_x(0.8 * swing * a.sin()), Vec3::ZERO),
            "lower_arm_l" | "lower_arm_r" => (rot_x(-0.5 - 0.3 * swing), Vec3::ZERO),
            _ => (Quat::IDENTITY, Vec3::ZERO),
        }
    })
}

fn idle(skeleton: &Skeleton) -> AnimationClip {
    make_clip(skeleton, "idle", 3.0, None, |bone, phase| {
        let a = TAU * phase;
        match bone {
            "spine" => (Quat::from_rotation_x(0.02 * a.sin()), Vec3::ZERO),
            "head" => (Quat::from_rotation_y(0.08 * (a * 0.5).sin()), Vec3::ZERO),
            "upper_arm_l" => (Quat::from_rotation_z(0.04 * a.sin() - 0.05), Vec3::ZERO),
            "upper_arm_r" => (Quat::from_rotation_z(-0.04 * a.sin() + 0.05), Vec3::ZERO),
            _ => (Quat::IDENTITY, Vec3::ZERO),
        }
    })
}

fn wave(skeleton: &Skeleton) -> AnimationClip {
    make_clip(
        skeleton,
        "wave",
        1.2,
        Some(&["upper_arm_r", "lower_arm_r"]),
        |bone, phase| {
            // Raise during the first and last 15 %, hold in between.
            let envelope = (phase / 0.15).min((1.0 - phase) / 0.15).clamp(0.0, 1.0);
            let eased = envelope * envelope * (3.0 - 2.0 * envelope);
            match bone {
                "upper_arm_r" => (Quat::from_rotation_z(2.4 * eased), Vec3::ZERO),
                _ => (
                    Quat::from_rotation_z(eased * 0.55 * (TAU * 3.0 * phase).sin()),
                    Vec3::ZERO,
                ),
            }
        },
    )
}

/// The skinned model with the clips [`IDLE`], [`WALK`], [`RUN`] and [`WAVE`].
pub fn humanoid() -> SkinnedModel {
    let skeleton = skeleton();
    let mut geometry = SkinnedGeometry::default();
    for (bone, from, to, r0, r1) in PARTS {
        add_part(
            &mut geometry,
            bone,
            Vec3::from(from),
            Vec3::from(to),
            r0,
            r1,
        );
    }
    let clips = vec![
        idle(&skeleton),
        locomotion(&skeleton, "walk", 1.0, 0.55, WALK_SPEED),
        locomotion(&skeleton, "run", 0.7, 0.95, RUN_SPEED * 0.7),
        wave(&skeleton),
    ];
    SkinnedModel {
        skeleton,
        clips,
        geometry,
    }
}

/// Idle / move (walk-run blend by `speed`) on the base layer; an upper-arm wave on a second layer
/// started by the `wave` trigger.
pub fn graph() -> AnimGraph {
    let speed = |compare, value| Condition::Float {
        param: "speed".into(),
        compare,
        value,
    };
    AnimGraph {
        root_bone: Some(ROOT),
        layers: vec![
            LayerDef {
                states: vec![
                    StateDef::new("idle", Motion::Clip(IDLE)),
                    StateDef::new(
                        "move",
                        Motion::Blend1D {
                            param: "speed".into(),
                            entries: vec![(0.0, WALK), (1.0, RUN)],
                        },
                    ),
                ],
                transitions: vec![
                    TransitionDef {
                        from: Some(0),
                        to: 1,
                        conditions: vec![speed(Compare::Greater, 0.1)],
                        duration: 0.2,
                        exit_time: None,
                    },
                    TransitionDef {
                        from: Some(1),
                        to: 0,
                        conditions: vec![speed(Compare::Less, 0.1)],
                        duration: 0.2,
                        exit_time: None,
                    },
                ],
                entry: 0,
                weight: 1.0,
                mask: None,
            },
            LayerDef {
                states: vec![
                    StateDef::new("rest", Motion::Clip(IDLE)),
                    StateDef::one_shot("wave", Motion::Clip(WAVE)),
                ],
                transitions: vec![
                    TransitionDef {
                        from: Some(0),
                        to: 1,
                        conditions: vec![Condition::Trigger {
                            param: "wave".into(),
                        }],
                        duration: 0.1,
                        exit_time: None,
                    },
                    TransitionDef {
                        from: Some(1),
                        to: 0,
                        conditions: vec![],
                        duration: 0.1,
                        exit_time: Some(1.0),
                    },
                ],
                entry: 0,
                weight: 0.0,
                mask: Some(vec!["upper_arm_r".into()]),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_is_consistent() {
        let model = humanoid();
        assert_eq!(model.skeleton.len(), 11);
        assert_eq!(model.clips.len(), 4);
        let g = &model.geometry;
        let n = g.positions.len();
        assert!(
            g.normals.len() == n && g.uvs.len() == n && g.joints.len() == n && g.weights.len() == n
        );
        assert!(g.indices.iter().all(|&i| (i as usize) < n));
        assert!(g.joints.iter().all(|j| (j[0] as usize) < 11));
        assert_eq!(g.indices.len() % 3, 0);
    }

    #[test]
    fn walk_and_run_travel_at_the_intended_speeds() {
        let model = humanoid();
        let z = |clip: usize, t: f32| {
            model.clips[clip].sample(&model.skeleton, t, false).bones[ROOT]
                .translation
                .z
        };
        let walk = &model.clips[WALK];
        let run = &model.clips[RUN];
        let walk_speed = (z(WALK, walk.duration) - z(WALK, 0.0)) / walk.duration;
        let run_speed = (z(RUN, run.duration) - z(RUN, 0.0)) / run.duration;
        assert!((walk_speed - WALK_SPEED).abs() < 1e-3, "{walk_speed}");
        assert!(
            (run_speed - RUN_SPEED * 0.7 / 0.7).abs() < 1e-3,
            "{run_speed}"
        );
    }
}

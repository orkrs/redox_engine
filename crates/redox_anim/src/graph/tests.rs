#![allow(clippy::unwrap_used)]

use redox_math::{Mat4, Quat};

use super::*;
use crate::Keyframe;

/// root -> spine -> arm. Clips move `root` along +x (walk/run) or turn the `arm`.
fn skeleton() -> Skeleton {
    Skeleton::from_local_bind(vec![
        ("root".into(), None, Mat4::IDENTITY),
        ("spine".into(), Some(0), Mat4::from_translation(Vec3::Y)),
        ("arm".into(), Some(1), Mat4::from_translation(Vec3::X)),
        ("leg".into(), Some(0), Mat4::from_translation(-Vec3::Y)),
    ])
}

fn key(time: f32, translation: Vec3, rotation: Quat) -> Keyframe {
    Keyframe {
        time,
        translation,
        rotation,
        scale: Vec3::ONE,
    }
}

/// Root travels `distance` along x in 1 second.
fn travel(name: &str, distance: f32) -> AnimationClip {
    let mut clip = AnimationClip {
        name: name.into(),
        duration: 1.0,
        ..Default::default()
    };
    clip.tracks.insert(
        "root".into(),
        vec![
            key(0.0, Vec3::ZERO, Quat::IDENTITY),
            key(1.0, Vec3::new(distance, 0.0, 0.0), Quat::IDENTITY),
        ],
    );
    clip
}

/// Arm rotates 90 degrees about z in 1 second.
fn wave() -> AnimationClip {
    let mut clip = AnimationClip {
        name: "wave".into(),
        duration: 1.0,
        ..Default::default()
    };
    clip.tracks.insert(
        "arm".into(),
        vec![
            key(0.0, Vec3::X, Quat::IDENTITY),
            key(
                1.0,
                Vec3::X,
                Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            ),
        ],
    );
    clip
}

fn clips() -> Vec<AnimationClip> {
    vec![
        travel("idle", 0.0),
        travel("walk", 2.0),
        travel("run", 6.0),
        wave(),
    ]
}

fn locomotion(root_bone: Option<usize>) -> AnimGraph {
    let go = |to, from, conds: Vec<Condition>, duration| TransitionDef {
        from: Some(from),
        to,
        conditions: conds,
        duration,
        exit_time: None,
    };
    AnimGraph {
        root_bone,
        layers: vec![LayerDef {
            states: vec![
                StateDef::new("idle", Motion::Clip(0)),
                StateDef::new(
                    "move",
                    Motion::Blend1D {
                        param: "speed".into(),
                        entries: vec![(0.0, 1), (1.0, 2)],
                    },
                ),
            ],
            transitions: vec![
                go(
                    1,
                    0,
                    vec![Condition::Float {
                        param: "speed".into(),
                        compare: Compare::Greater,
                        value: 0.1,
                    }],
                    0.2,
                ),
                go(
                    0,
                    1,
                    vec![Condition::Float {
                        param: "speed".into(),
                        compare: Compare::Less,
                        value: 0.1,
                    }],
                    0.2,
                ),
            ],
            entry: 0,
            weight: 1.0,
            mask: None,
        }],
    }
}

fn root_x(pose: &LocalPose) -> f32 {
    pose.bones[0].translation.x
}

#[test]
fn blend_tree_mixes_clips_by_the_parameter() {
    let skeleton = skeleton();
    let graph = AnimGraph {
        root_bone: None,
        layers: vec![LayerDef {
            states: vec![StateDef::new(
                "move",
                Motion::Blend1D {
                    param: "speed".into(),
                    entries: vec![(0.0, 1), (1.0, 2)],
                },
            )],
            transitions: vec![],
            entry: 0,
            weight: 1.0,
            mask: None,
        }],
    };
    let mut player = GraphPlayer::new(graph, &skeleton);
    player.params.set_float("speed", 0.5);
    let out = player.update(0.5, &skeleton, &clips());
    // Halfway through the phase; walk would be at 1.0, run at 3.0, the mix at 2.0.
    assert!(
        (root_x(&out.pose) - 2.0).abs() < 1e-4,
        "{}",
        root_x(&out.pose)
    );
    player.params.set_float("speed", 5.0);
    let out = player.update(0.0, &skeleton, &clips());
    assert!(
        (root_x(&out.pose) - 3.0).abs() < 1e-4,
        "clamps to the last entry"
    );
}

#[test]
fn a_transition_waits_for_its_condition_then_cross_fades() {
    let skeleton = skeleton();
    let mut player = GraphPlayer::new(locomotion(None), &skeleton);
    for _ in 0..5 {
        player.update(0.1, &skeleton, &clips());
    }
    assert_eq!(player.state_name(0), Some("idle"));
    assert!(!player.is_fading(0));

    player.params.set_float("speed", 1.0);
    let out = player.update(0.1, &skeleton, &clips());
    assert!(
        player.is_fading(0),
        "started fading as soon as the condition held"
    );
    assert_eq!(out.events, vec!["move".to_string()]);
    assert_eq!(player.state_name(0), Some("move"), "reports the target");
    player.update(0.1, &skeleton, &clips());
    player.update(0.1, &skeleton, &clips());
    assert!(!player.is_fading(0), "fade of 0.2 s is over");
    assert_eq!(player.state_name(0), Some("move"));
}

#[test]
fn exit_time_holds_a_transition_back() {
    let skeleton = skeleton();
    let mut graph = locomotion(None);
    graph.layers[0].states[0] = StateDef::new("idle", Motion::Clip(0));
    graph.layers[0].transitions[0].exit_time = Some(0.75);
    let mut player = GraphPlayer::new(graph, &skeleton);
    player.params.set_float("speed", 1.0);
    player.update(0.5, &skeleton, &clips());
    assert!(!player.is_fading(0), "phase 0.5 < exit time 0.75");
    player.update(0.3, &skeleton, &clips());
    player.update(0.0, &skeleton, &clips());
    assert!(player.is_fading(0), "phase 0.8 >= 0.75");
}

#[test]
fn triggers_fire_once_and_are_consumed() {
    let skeleton = skeleton();
    let graph = AnimGraph {
        root_bone: None,
        layers: vec![LayerDef {
            states: vec![
                StateDef::new("idle", Motion::Clip(0)),
                StateDef::one_shot("wave", Motion::Clip(3)),
            ],
            transitions: vec![
                TransitionDef {
                    from: None,
                    to: 1,
                    conditions: vec![Condition::Trigger {
                        param: "wave".into(),
                    }],
                    duration: 0.0,
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
            weight: 1.0,
            mask: None,
        }],
    };
    let mut player = GraphPlayer::new(graph, &skeleton);
    player.params.trigger("wave");
    player.update(0.1, &skeleton, &clips());
    assert_eq!(player.state_name(0), Some("wave"));
    assert!(!player.params.triggered("wave"), "consumed");
    // Play the one-shot to its end, then it returns to idle by itself.
    for _ in 0..14 {
        player.update(0.1, &skeleton, &clips());
    }
    assert_eq!(player.state_name(0), Some("idle"));
}

#[test]
fn a_masked_layer_only_drives_its_bones_and_their_children() {
    let skeleton = skeleton();
    let upper = LayerDef {
        states: vec![StateDef::new("wave", Motion::Clip(3))],
        transitions: vec![],
        entry: 0,
        weight: 1.0,
        mask: Some(vec!["spine".into()]),
    };
    let mut graph = locomotion(None);
    graph.layers.push(upper);
    let mut player = GraphPlayer::new(graph, &skeleton);
    let out = player.update(1.0 - 1e-3, &skeleton, &clips());
    let arm = skeleton.bone_index("arm").unwrap();
    let leg = skeleton.bone_index("leg").unwrap();
    let angle = out.pose.bones[arm].rotation.angle_between(Quat::IDENTITY);
    assert!(
        angle > 1.5,
        "arm (child of spine) follows the layer: {angle}"
    );
    assert!(
        out.pose.bones[leg].rotation.angle_between(Quat::IDENTITY) < 1e-4,
        "leg is outside the mask"
    );
    player.set_layer_weight(1, 0.0);
    let out = player.update(0.0, &skeleton, &clips());
    assert!(out.pose.bones[arm].rotation.angle_between(Quat::IDENTITY) < 1e-4);
}

#[test]
fn root_motion_is_removed_from_the_pose_and_reported_per_frame() {
    let skeleton = skeleton();
    let mut graph = locomotion(Some(0));
    // Start in the move state at speed 0.2: walk/run mix, 2 + 0.2 * 4 = 2.8 m per second.
    graph.layers[0].entry = 1;
    let mut player = GraphPlayer::new(graph, &skeleton);
    player.params.set_float("speed", 0.2);
    let mut total = 0.0;
    for _ in 0..10 {
        let out = player.update(0.05, &skeleton, &clips());
        assert!(root_x(&out.pose).abs() < 1e-5, "pose stays in place");
        total += out.root_motion.translation.x;
        assert_eq!(out.root_motion.translation.y, 0.0);
    }
    assert!(
        (total - 1.4).abs() < 1e-3,
        "0.5 s at 2.8 m/s is 1.4 m, got {total}"
    );
}

#[test]
fn root_motion_survives_the_loop_wrap() {
    let skeleton = skeleton();
    let mut graph = locomotion(Some(0));
    graph.layers[0].entry = 1;
    let mut player = GraphPlayer::new(graph, &skeleton);
    player.params.set_float("speed", 0.2);
    let mut total = 0.0;
    for _ in 0..30 {
        total += player
            .update(0.1, &skeleton, &clips())
            .root_motion
            .translation
            .x;
    }
    // 3 s at 2.8 m per second, across three wraps.
    assert!((total - 8.4).abs() < 1e-2, "{total}");
}

#[test]
fn missing_clips_and_states_fall_back_to_the_bind_pose() {
    let skeleton = skeleton();
    let mut graph = locomotion(None);
    graph.layers[0].entry = 9;
    let mut player = GraphPlayer::new(graph, &skeleton);
    let out = player.update(0.1, &skeleton, &[]);
    assert_eq!(out.pose, skeleton.bind_local_pose());
    let empty = GraphPlayer::new(AnimGraph::default(), &skeleton).update(0.1, &skeleton, &[]);
    assert_eq!(empty.pose, skeleton.bind_local_pose());
}

#[test]
fn graphs_round_trip_through_serde() {
    let graph = locomotion(Some(0));
    let text = ron::to_string(&graph).unwrap();
    let back: AnimGraph = ron::from_str(&text).unwrap();
    assert_eq!(back.layers.len(), 1);
    assert_eq!(back.layers[0].states[1].name, "move");
    assert_eq!(back.layers[0].transitions.len(), 2);
    assert_eq!(back.root_bone, Some(0));
}

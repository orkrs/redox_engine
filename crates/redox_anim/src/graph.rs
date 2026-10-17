//! Animation graph: parameters, state machines with timed transitions, 1D blend trees with phase
//! sync, masked layers, root motion and state-enter events.
//!
//! A graph is plain data ([`AnimGraph`], serde-friendly, built in code or loaded from a file).
//! A [`GraphPlayer`] owns the runtime state (parameters, current states, transitions in
//! progress) and turns it into a [`LocalPose`] each frame. Pose evaluation reads clips from the
//! model, so one graph can drive any skeleton whose clips it names by index.
//!
//! * **States** play a [`Motion`]: one clip, or a blend tree that mixes clips by a float parameter.
//!   Clips of a blend tree share one normalised phase, so footsteps stay in sync.
//! * **Transitions** fire when all their [`Condition`]s hold (and an optional exit time has been
//!   reached) and cross-fade over a duration.
//! * **Layers** are extra state machines blended over the base one, optionally limited to some
//!   bones (and their children) by a mask: an upper-body attack over a running lower body.
//! * **Root motion**: the horizontal movement of the root bone is removed from the pose and
//!   returned as a per-frame delta for the gameplay code to apply to the character.

use std::collections::HashMap;

use redox_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::{AnimationClip, BoneTransform, LocalPose, Skeleton};

/// Value of a named graph parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    /// A number (speed, direction, ...).
    Float(f32),
    /// An on/off switch (grounded, aiming, ...).
    Bool(bool),
    /// A one-shot signal that is cleared when a transition consumes it.
    Trigger(bool),
}

/// Named parameters that conditions and blend trees read.
#[derive(Clone, Debug, Default)]
pub struct Params {
    values: HashMap<String, ParamValue>,
}

impl Params {
    /// Sets a float parameter.
    pub fn set_float(&mut self, name: &str, value: f32) {
        self.values.insert(name.into(), ParamValue::Float(value));
    }

    /// Sets a bool parameter.
    pub fn set_bool(&mut self, name: &str, value: bool) {
        self.values.insert(name.into(), ParamValue::Bool(value));
    }

    /// Raises a trigger; the first transition that waits for it clears it.
    pub fn trigger(&mut self, name: &str) {
        self.values.insert(name.into(), ParamValue::Trigger(true));
    }

    /// A float parameter (0 when missing or of another kind).
    pub fn float(&self, name: &str) -> f32 {
        match self.values.get(name) {
            Some(ParamValue::Float(v)) => *v,
            _ => 0.0,
        }
    }

    /// A bool parameter (false when missing or of another kind).
    pub fn bool(&self, name: &str) -> bool {
        matches!(self.values.get(name), Some(ParamValue::Bool(true)))
    }

    fn triggered(&self, name: &str) -> bool {
        matches!(self.values.get(name), Some(ParamValue::Trigger(true)))
    }

    fn clear_trigger(&mut self, name: &str) {
        if let Some(ParamValue::Trigger(on)) = self.values.get_mut(name) {
            *on = false;
        }
    }
}

/// What a state plays.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Motion {
    /// One clip, by index into the model's clips.
    Clip(usize),
    /// Clips mixed by a float parameter. `entries` are `(threshold, clip)` sorted by threshold.
    Blend1D {
        /// Parameter that selects the mix.
        param: String,
        /// `(threshold, clip index)` pairs, ascending.
        entries: Vec<(f32, usize)>,
    },
}

/// Comparison used by a float condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compare {
    /// Parameter is greater than the value.
    Greater,
    /// Parameter is less than the value.
    Less,
}

/// A requirement of a transition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Condition {
    /// Float parameter compared with a value.
    Float {
        /// Parameter name.
        param: String,
        /// How to compare.
        compare: Compare,
        /// Value to compare with.
        value: f32,
    },
    /// Bool parameter equals a value.
    Bool {
        /// Parameter name.
        param: String,
        /// Required value.
        value: bool,
    },
    /// Trigger parameter is raised (it is cleared when the transition fires).
    Trigger {
        /// Parameter name.
        param: String,
    },
}

/// One state of a machine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateDef {
    /// Name (reported in enter events).
    pub name: String,
    /// What the state plays.
    pub motion: Motion,
    /// Playback speed multiplier.
    pub speed: f32,
    /// Whether the phase wraps; a one-shot stops on its last frame.
    pub looping: bool,
}

impl StateDef {
    /// A looping state at normal speed.
    pub fn new(name: &str, motion: Motion) -> Self {
        Self {
            name: name.into(),
            motion,
            speed: 1.0,
            looping: true,
        }
    }

    /// A state that plays once and holds its last frame.
    pub fn one_shot(name: &str, motion: Motion) -> Self {
        Self {
            looping: false,
            ..Self::new(name, motion)
        }
    }
}

/// A switch from one state to another.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransitionDef {
    /// Source state, or `None` to leave from any state.
    pub from: Option<usize>,
    /// Target state.
    pub to: usize,
    /// All of these must hold.
    pub conditions: Vec<Condition>,
    /// Cross-fade length in seconds (0 = instant).
    pub duration: f32,
    /// Normalised phase (0..1) the source must have reached; `None` = any time.
    pub exit_time: Option<f32>,
}

/// One state machine blended into the result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerDef {
    /// States.
    pub states: Vec<StateDef>,
    /// Transitions between them.
    pub transitions: Vec<TransitionDef>,
    /// State the layer starts in.
    pub entry: usize,
    /// Blend weight over the layers below (0..1).
    pub weight: f32,
    /// Bone names (with their children) the layer drives; `None` = the whole skeleton.
    pub mask: Option<Vec<String>>,
}

/// A complete graph. Layer 0 is the base layer and the only one that produces root motion.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnimGraph {
    /// Layers, base first.
    pub layers: Vec<LayerDef>,
    /// Index of the root bone whose horizontal movement becomes root motion (`None` = off).
    pub root_bone: Option<usize>,
}

/// Movement taken out of the animation this frame, to be applied to the character.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RootMotion {
    /// Horizontal displacement (x, z; y is always 0) in the root bone's parent space.
    pub translation: Vec3,
}

/// Result of one [`GraphPlayer::update`].
#[derive(Clone, Debug)]
pub struct GraphOutput {
    /// The pose to skin.
    pub pose: LocalPose,
    /// Movement removed from the root bone.
    pub root_motion: RootMotion,
    /// Names of states entered this frame (a transition counts when it starts).
    pub events: Vec<String>,
}

#[derive(Clone, Debug)]
struct Fade {
    to: usize,
    to_phase: f32,
    elapsed: f32,
    duration: f32,
}

#[derive(Clone, Debug)]
struct LayerState {
    current: usize,
    phase: f32,
    fade: Option<Fade>,
    bone_weights: Vec<f32>,
}

/// Runtime of an [`AnimGraph`].
#[derive(Clone, Debug)]
pub struct GraphPlayer {
    graph: AnimGraph,
    /// Parameters the game sets each frame.
    pub params: Params,
    layers: Vec<LayerState>,
    events: Vec<String>,
}

fn motion_duration(motion: &Motion, params: &Params, clips: &[AnimationClip]) -> f32 {
    let clip_len = |i: usize| clips.get(i).map_or(0.0, |c| c.duration);
    match motion {
        Motion::Clip(i) => clip_len(*i),
        Motion::Blend1D { param, entries } => {
            let (a, b, t) = blend_pair(entries, params.float(param));
            clip_len(a) + (clip_len(b) - clip_len(a)) * t
        }
    }
}

/// The two clips around `value` and the weight of the second.
fn blend_pair(entries: &[(f32, usize)], value: f32) -> (usize, usize, f32) {
    match entries {
        [] => (usize::MAX, usize::MAX, 0.0),
        [only] => (only.1, only.1, 0.0),
        _ => {
            let next = entries.partition_point(|(threshold, _)| *threshold <= value);
            if next == 0 {
                (entries[0].1, entries[0].1, 0.0)
            } else if next >= entries.len() {
                let last = entries[entries.len() - 1].1;
                (last, last, 0.0)
            } else {
                let (t0, c0) = entries[next - 1];
                let (t1, c1) = entries[next];
                (
                    c0,
                    c1,
                    ((value - t0) / (t1 - t0).max(f32::EPSILON)).clamp(0.0, 1.0),
                )
            }
        }
    }
}

fn sample_clip(
    clips: &[AnimationClip],
    index: usize,
    skeleton: &Skeleton,
    phase: f32,
    looping: bool,
) -> LocalPose {
    match clips.get(index) {
        Some(clip) => clip.sample(skeleton, phase * clip.duration, looping),
        None => skeleton.bind_local_pose(),
    }
}

fn sample_motion(
    motion: &Motion,
    params: &Params,
    clips: &[AnimationClip],
    skeleton: &Skeleton,
    phase: f32,
    looping: bool,
) -> LocalPose {
    match motion {
        Motion::Clip(i) => sample_clip(clips, *i, skeleton, phase, looping),
        Motion::Blend1D { param, entries } => {
            let (a, b, t) = blend_pair(entries, params.float(param));
            let pose_a = sample_clip(clips, a, skeleton, phase, looping);
            if t <= 0.0 || a == b {
                pose_a
            } else {
                pose_a.blend(&sample_clip(clips, b, skeleton, phase, looping), t)
            }
        }
    }
}

/// Horizontal root position of a motion at `phase`.
fn root_position(
    motion: &Motion,
    params: &Params,
    clips: &[AnimationClip],
    skeleton: &Skeleton,
    root: usize,
    phase: f32,
    looping: bool,
) -> Vec3 {
    let pose = sample_motion(motion, params, clips, skeleton, phase, looping);
    pose.bones.get(root).map_or(Vec3::ZERO, |b| {
        Vec3::new(b.translation.x, 0.0, b.translation.z)
    })
}

/// Root displacement of a motion between two phases (handles a loop wrap).
#[allow(clippy::too_many_arguments)]
fn root_delta(
    state: &StateDef,
    params: &Params,
    clips: &[AnimationClip],
    skeleton: &Skeleton,
    root: usize,
    from: f32,
    to: f32,
    wrapped: bool,
) -> Vec3 {
    let at = |phase, looping| {
        root_position(&state.motion, params, clips, skeleton, root, phase, looping)
    };
    if wrapped {
        // The end of the clip is sampled without looping: with it, phase 1.0 would be the start.
        (at(1.0, false) - at(from, state.looping))
            + (at(to, state.looping) - at(0.0, state.looping))
    } else {
        at(to, state.looping) - at(from, state.looping)
    }
}

impl GraphPlayer {
    /// Starts every layer in its entry state.
    pub fn new(graph: AnimGraph, skeleton: &Skeleton) -> Self {
        let layers = graph
            .layers
            .iter()
            .map(|layer| LayerState {
                current: layer.entry,
                phase: 0.0,
                fade: None,
                bone_weights: mask_weights(layer, skeleton),
            })
            .collect();
        let events = graph
            .layers
            .iter()
            .filter_map(|l| l.states.get(l.entry).map(|s| s.name.clone()))
            .collect();
        Self {
            graph,
            params: Params::default(),
            layers,
            events,
        }
    }

    /// Name of the state a layer is in (the target while a fade is running).
    pub fn state_name(&self, layer: usize) -> Option<&str> {
        let state = self.layers.get(layer)?;
        let index = state.fade.as_ref().map_or(state.current, |f| f.to);
        self.graph
            .layers
            .get(layer)?
            .states
            .get(index)
            .map(|s| s.name.as_str())
    }

    /// Whether a layer is cross-fading between two states.
    pub fn is_fading(&self, layer: usize) -> bool {
        self.layers.get(layer).is_some_and(|l| l.fade.is_some())
    }

    /// Sets a layer's blend weight (0..1).
    pub fn set_layer_weight(&mut self, layer: usize, weight: f32) {
        if let Some(def) = self.graph.layers.get_mut(layer) {
            def.weight = weight.clamp(0.0, 1.0);
        }
    }

    /// Advances every layer by `dt` and returns the blended pose.
    pub fn update(&mut self, dt: f32, skeleton: &Skeleton, clips: &[AnimationClip]) -> GraphOutput {
        let mut events = std::mem::take(&mut self.events);
        let mut result: Option<LocalPose> = None;
        let mut root_motion = RootMotion::default();

        for index in 0..self.layers.len() {
            let (pose, delta) = self.update_layer(index, dt, skeleton, clips, &mut events);
            if index == 0 {
                root_motion.translation = delta;
            }
            let def = &self.graph.layers[index];
            result = Some(match result {
                None => pose,
                Some(base) => {
                    blend_masked(&base, &pose, def.weight, &self.layers[index].bone_weights)
                }
            });
        }

        let mut pose = result.unwrap_or_else(|| skeleton.bind_local_pose());
        if let Some(root) = self.graph.root_bone
            && let (Some(bone), Some(bind)) = (pose.bones.get_mut(root), skeleton.bones.get(root))
        {
            let bind = BoneTransform::from_mat4(bind.bind_pose);
            bone.translation.x = bind.translation.x;
            bone.translation.z = bind.translation.z;
        }
        GraphOutput {
            pose,
            root_motion,
            events,
        }
    }

    fn update_layer(
        &mut self,
        index: usize,
        dt: f32,
        skeleton: &Skeleton,
        clips: &[AnimationClip],
        events: &mut Vec<String>,
    ) -> (LocalPose, Vec3) {
        self.try_start_transition(index, events);

        let def = &self.graph.layers[index];
        let root = if index == 0 {
            self.graph.root_bone
        } else {
            None
        };
        let state = &mut self.layers[index];
        let params = &self.params;

        let advance = |state_def: &StateDef, phase: f32| -> (f32, bool) {
            let duration = motion_duration(&state_def.motion, params, clips).max(f32::EPSILON);
            let next = phase + dt * state_def.speed / duration;
            if state_def.looping {
                (next.rem_euclid(1.0), next >= 1.0)
            } else {
                (next.min(1.0), false)
            }
        };

        let Some(current_def) = def.states.get(state.current) else {
            return (skeleton.bind_local_pose(), Vec3::ZERO);
        };
        let from_phase = state.phase;
        let (new_phase, wrapped) = advance(current_def, from_phase);
        state.phase = new_phase;
        let mut delta = root.map_or(Vec3::ZERO, |r| {
            root_delta(
                current_def,
                params,
                clips,
                skeleton,
                r,
                from_phase,
                new_phase,
                wrapped,
            )
        });
        let current_pose = sample_motion(
            &current_def.motion,
            params,
            clips,
            skeleton,
            new_phase,
            current_def.looping,
        );

        let Some(fade) = state.fade.as_mut() else {
            return (current_pose, delta);
        };
        let Some(target_def) = def.states.get(fade.to) else {
            state.fade = None;
            return (current_pose, delta);
        };
        let target_from = fade.to_phase;
        let (target_phase, target_wrapped) = advance(target_def, target_from);
        fade.to_phase = target_phase;
        fade.elapsed += dt;
        let weight = if fade.duration <= f32::EPSILON {
            1.0
        } else {
            (fade.elapsed / fade.duration).clamp(0.0, 1.0)
        };
        let target_pose = sample_motion(
            &target_def.motion,
            params,
            clips,
            skeleton,
            target_phase,
            target_def.looping,
        );
        if let Some(r) = root {
            let target_delta = root_delta(
                target_def,
                params,
                clips,
                skeleton,
                r,
                target_from,
                target_phase,
                target_wrapped,
            );
            delta = delta.lerp(target_delta, weight);
        }
        let pose = current_pose.blend(&target_pose, weight);
        if weight >= 1.0 {
            state.current = fade.to;
            state.phase = fade.to_phase;
            state.fade = None;
        }
        (pose, delta)
    }

    fn try_start_transition(&mut self, index: usize, events: &mut Vec<String>) {
        if self.layers[index].fade.is_some() {
            return;
        }
        let def = &self.graph.layers[index];
        let current = self.layers[index].current;
        let phase = self.layers[index].phase;
        let looping = def.states.get(current).is_some_and(|s| s.looping);
        let chosen = def.transitions.iter().find(|t| {
            t.to != current
                && t.from.is_none_or(|from| from == current)
                && t.exit_time
                    .is_none_or(|exit| phase >= exit || (!looping && phase >= 1.0 - f32::EPSILON))
                && t.conditions.iter().all(|c| self.holds(c))
        });
        let Some(transition) = chosen else {
            return;
        };
        for condition in &transition.conditions {
            if let Condition::Trigger { param } = condition {
                self.params.clear_trigger(param);
            }
        }
        if let Some(target) = def.states.get(transition.to) {
            events.push(target.name.clone());
        }
        let (to, duration) = (transition.to, transition.duration.max(0.0));
        self.layers[index].fade = Some(Fade {
            to,
            to_phase: 0.0,
            elapsed: 0.0,
            duration,
        });
    }

    fn holds(&self, condition: &Condition) -> bool {
        match condition {
            Condition::Float {
                param,
                compare,
                value,
            } => {
                let v = self.params.float(param);
                match compare {
                    Compare::Greater => v > *value,
                    Compare::Less => v < *value,
                }
            }
            Condition::Bool { param, value } => self.params.bool(param) == *value,
            Condition::Trigger { param } => self.params.triggered(param),
        }
    }
}

/// Per-bone weights of a layer: 1 for the masked bones and their children, 0 elsewhere.
fn mask_weights(layer: &LayerDef, skeleton: &Skeleton) -> Vec<f32> {
    let Some(names) = &layer.mask else {
        return vec![1.0; skeleton.len()];
    };
    let mut weights = vec![0.0; skeleton.len()];
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let mut cursor = Some(i);
        while let Some(c) = cursor {
            if names.iter().any(|n| *n == skeleton.bones[c].name) {
                weights[i] = 1.0;
                break;
            }
            cursor = skeleton.bones[c].parent;
        }
        let _ = bone;
    }
    weights
}

fn blend_masked(base: &LocalPose, top: &LocalPose, weight: f32, mask: &[f32]) -> LocalPose {
    let bones = base
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let w = weight.clamp(0.0, 1.0) * mask.get(i).copied().unwrap_or(1.0);
            match top.bones.get(i) {
                Some(t) if w > 0.0 => b.lerp(*t, w),
                _ => *b,
            }
        })
        .collect();
    LocalPose { bones }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
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
}

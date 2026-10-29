//! Binding of an [`AnimGraph`] to a glTF model (ANM-04).
//!
//! A graph refers to clips only by index ([`Motion`] stores `usize`), so the names live outside it:
//! a [`ClipRole`] says what the game calls a clip ("idle", "walk", ...) and which glTF clip names may
//! supply it. [`bind_clips`] turns the roles into indices of the model's clips, and
//! [`validate_binding`] checks that every index and every bone the graph uses exists.

use std::collections::{BTreeSet, HashMap};

use crate::{AnimGraph, Motion, Skeleton, SkinnedModel};

/// A gameplay clip role and the glTF clip names that may supply it, in order of preference.
#[derive(Clone, Copy, Debug)]
pub struct ClipRole {
    /// Role name used by the game, e.g. `"idle"`.
    pub role: &'static str,
    /// Clip names of the model that can play this role, tried in order.
    pub candidates: &'static [&'static str],
    /// Whether a missing clip is an error (`false` = the game disables what uses this role).
    pub required: bool,
}

/// Role name to index into the model's clips, produced by [`bind_clips`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipBinding {
    indices: HashMap<&'static str, usize>,
}

impl ClipBinding {
    /// Index of the model clip bound to `role`, or `None` when the role is absent.
    pub fn index(&self, role: &str) -> Option<usize> {
        self.indices.get(role).copied()
    }

    /// Number of bound roles.
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether no role is bound.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// A problem found while binding a graph to a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingError {
    /// A required clip role has no matching clip in the model.
    MissingClip {
        /// The role that could not be bound.
        role: String,
    },
    /// A bone named by a layer mask is not in the skeleton.
    MissingBone(String),
    /// A state refers to a clip index that the model does not have.
    ClipIndexOutOfRange {
        /// Name of the state that refers to the clip.
        state: String,
        /// The offending clip index.
        index: usize,
    },
}

/// Matches each role to a clip of the model and returns all unmatched required roles at once.
///
/// Names are compared case-insensitively, ignoring an `armature|`-style prefix (everything up to the
/// last `|`). An override `(role, clip name)` wins over the candidates; if the overridden clip is not
/// in the model, the role falls back to its candidates.
pub fn bind_clips(
    roles: &[ClipRole],
    model: &SkinnedModel,
    overrides: &[(&str, &str)],
) -> Result<ClipBinding, Vec<BindingError>> {
    let mut indices = HashMap::new();
    let mut errors = Vec::new();
    for role in roles {
        match find_clip(role, model, overrides) {
            Some(index) => {
                indices.insert(role.role, index);
            }
            None if role.required => errors.push(BindingError::MissingClip {
                role: role.role.to_string(),
            }),
            None => {}
        }
    }
    if errors.is_empty() {
        Ok(ClipBinding { indices })
    } else {
        Err(errors)
    }
}

fn find_clip(role: &ClipRole, model: &SkinnedModel, overrides: &[(&str, &str)]) -> Option<usize> {
    let by_name = |wanted: &str| {
        let wanted = normalize(wanted);
        model
            .clips
            .iter()
            .position(|clip| normalize(&clip.name) == wanted)
    };
    overrides
        .iter()
        .find(|entry| entry.0 == role.role)
        .and_then(|entry| by_name(entry.1))
        .or_else(|| role.candidates.iter().find_map(|name| by_name(name)))
}

/// Lower-cased name without an `armature|` style prefix.
fn normalize(name: &str) -> String {
    let base = name.rsplit('|').next().unwrap_or(name);
    base.trim().to_lowercase()
}

/// Names of the bones that the graph's layer masks reference, sorted and without duplicates.
pub fn required_bones(graph: &AnimGraph) -> Vec<String> {
    let names: BTreeSet<String> = graph
        .layers
        .iter()
        .filter_map(|layer| layer.mask.as_ref())
        .flatten()
        .cloned()
        .collect();
    names.into_iter().collect()
}

/// Checks the graph against a model: every clip index used by a state (directly or in a blend tree)
/// must be below `clips_len`, and every bone of [`required_bones`] must exist in `skeleton`.
/// Reports all problems, not only the first one.
pub fn validate_binding(
    graph: &AnimGraph,
    skeleton: &Skeleton,
    clips_len: usize,
) -> Result<(), Vec<BindingError>> {
    let mut errors = Vec::new();
    for layer in &graph.layers {
        for state in &layer.states {
            let indices: Vec<usize> = match &state.motion {
                Motion::Clip(index) => vec![*index],
                Motion::Blend1D { entries, .. } => {
                    entries.iter().map(|(_, index)| *index).collect()
                }
            };
            for index in indices.into_iter().filter(|index| *index >= clips_len) {
                errors.push(BindingError::ClipIndexOutOfRange {
                    state: state.name.clone(),
                    index,
                });
            }
        }
    }
    for bone in required_bones(graph) {
        if skeleton.bone_index(&bone).is_none() {
            errors.push(BindingError::MissingBone(bone));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnimationClip, LayerDef, StateDef};
    use redox_math::Mat4;

    const IDLE: ClipRole = ClipRole {
        role: "idle",
        candidates: &["idle", "stand"],
        required: true,
    };
    const WAVE: ClipRole = ClipRole {
        role: "wave",
        candidates: &["wave"],
        required: false,
    };

    /// A model with the given bones (a chain) and clips of duration one second.
    fn model(bones: &[&str], clips: &[&str]) -> SkinnedModel {
        let bones = bones
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let parent = if i == 0 { None } else { Some(i - 1) };
                ((*name).to_string(), parent, Mat4::IDENTITY)
            })
            .collect();
        SkinnedModel {
            skeleton: Skeleton::from_local_bind(bones),
            clips: clips
                .iter()
                .map(|name| AnimationClip {
                    name: (*name).to_string(),
                    duration: 1.0,
                    ..Default::default()
                })
                .collect(),
            geometry: Default::default(),
        }
    }

    fn graph(states: Vec<StateDef>, mask: Option<Vec<String>>) -> AnimGraph {
        AnimGraph {
            layers: vec![LayerDef {
                states,
                transitions: Vec::new(),
                entry: 0,
                weight: 1.0,
                mask,
            }],
            root_bone: None,
        }
    }

    #[test]
    fn a_complete_model_binds_all_roles() {
        let m = model(&["root"], &["idle", "wave"]);
        let binding = bind_clips(&[IDLE, WAVE], &m, &[]).expect("binds");
        assert_eq!(binding.index("idle"), Some(0));
        assert_eq!(binding.index("wave"), Some(1));
        assert_eq!(binding.len(), 2);
    }

    #[test]
    fn missing_required_roles_are_all_reported() {
        let m = model(&["root"], &[]);
        let Err(errors) = bind_clips(&[IDLE, WAVE], &m, &[]) else {
            panic!("idle is required");
        };
        assert_eq!(
            errors,
            vec![BindingError::MissingClip {
                role: "idle".into()
            }]
        );
    }

    #[test]
    fn an_optional_role_may_be_absent() {
        let m = model(&["root"], &["idle"]);
        let binding = bind_clips(&[IDLE, WAVE], &m, &[]).expect("binds");
        assert_eq!(binding.index("wave"), None);
        assert!(!binding.is_empty());
    }

    #[test]
    fn names_match_case_insensitively_and_ignore_the_armature_prefix() {
        let m = model(&["root"], &["Armature|IDLE"]);
        let binding = bind_clips(&[IDLE], &m, &[]).expect("binds");
        assert_eq!(binding.index("idle"), Some(0));
    }

    #[test]
    fn an_override_wins_over_the_candidates() {
        let m = model(&["root"], &["idle", "Breathe"]);
        let binding = bind_clips(&[IDLE], &m, &[("idle", "Breathe")]).expect("binds");
        assert_eq!(binding.index("idle"), Some(1));
    }

    #[test]
    fn candidates_are_tried_in_order() {
        let roles = [ClipRole {
            role: "walk",
            candidates: &["walk_loop", "walk"],
            required: true,
        }];
        let m = model(&["root"], &["walk", "walk_loop"]);
        let binding = bind_clips(&roles, &m, &[]).expect("binds");
        assert_eq!(binding.index("walk"), Some(1));
    }

    #[test]
    fn required_bones_come_from_layer_masks() {
        let g = graph(
            vec![StateDef::new("idle", Motion::Clip(0))],
            Some(vec!["spine".into(), "hand_R".into(), "spine".into()]),
        );
        assert_eq!(required_bones(&g), vec!["hand_R", "spine"]);
    }

    #[test]
    fn validate_binding_reports_an_out_of_range_clip_index_and_a_missing_bone() {
        let g = graph(
            vec![
                StateDef::new("idle", Motion::Clip(0)),
                StateDef::new(
                    "walk",
                    Motion::Blend1D {
                        param: "speed".into(),
                        entries: vec![(0.0, 0), (1.0, 3)],
                    },
                ),
            ],
            Some(vec!["spine".into()]),
        );
        let m = model(&["root"], &["idle", "walk"]);
        let Err(errors) = validate_binding(&g, &m.skeleton, m.clips.len()) else {
            panic!("binding is broken");
        };
        assert_eq!(
            errors,
            vec![
                BindingError::ClipIndexOutOfRange {
                    state: "walk".into(),
                    index: 3
                },
                BindingError::MissingBone("spine".into()),
            ]
        );
    }

    #[test]
    fn a_valid_graph_validates() {
        let g = graph(vec![StateDef::new("idle", Motion::Clip(0))], None);
        let m = model(&["root"], &["idle"]);
        assert!(validate_binding(&g, &m.skeleton, m.clips.len()).is_ok());
    }
}

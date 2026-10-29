//! The Foxfire fox as Courier's character (ANM-04): its glTF skeleton and clips, bound to the game's
//! clip roles and drawn at the same scale as in Foxfire. Without a wave clip the arm layer is off.
//! The fox has no root motion, so the game moves it (see `game::World::step`).

use std::path::Path;

use redox_anim::gltf_import::load_glb;
use redox_anim::{bind_clips, validate_binding, ClipRole};

use crate::rig::{self, Rig};

/// Scale from the fox model's units (about 80 tall) to metres. Same value as `MODEL_SCALE` in
/// `examples/foxrun/fox.rs`; keep them in step.
pub const MODEL_SCALE: f32 = 0.012;

/// The fox file shared with Foxfire.
pub const FOX_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/foxrun/Fox.glb");

/// Clip roles of the fox: Survey is the idle clip, and the fox has no wave.
pub const FOX_ROLES: [ClipRole; 4] = [
    ClipRole {
        role: "idle",
        candidates: &["survey", "idle"],
        required: true,
    },
    ClipRole {
        role: "walk",
        candidates: &["walk"],
        required: true,
    },
    ClipRole {
        role: "run",
        candidates: &["run"],
        required: true,
    },
    ClipRole {
        role: "wave",
        candidates: &["wave"],
        required: false,
    },
];

/// Loads the fox from [`FOX_PATH`].
pub fn load() -> Result<Rig, String> {
    load_from(Path::new(FOX_PATH))
}

/// Loads a glTF file, binds the fox roles, builds the graph and validates it against the model.
pub fn load_from(path: &Path) -> Result<Rig, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let model = load_glb(&bytes)?
        .into_iter()
        .next()
        .ok_or("no skinned model in the file")?;
    let binding = bind_clips(&FOX_ROLES, &model, &[]).map_err(|e| format!("{e:?}"))?;
    let graph = rig::graph(&binding, None).ok_or("the fox lacks an idle, walk or run clip")?;
    validate_binding(&graph, &model.skeleton, model.clips.len())
        .map_err(|e| format!("{e:?}"))?;
    Ok(Rig {
        model,
        graph,
        scale: MODEL_SCALE,
    })
}

#[cfg(test)]
mod tests {
    use redox_anim::graph::GraphPlayer;

    use super::*;

    #[test]
    fn courier_uses_the_gltf_clips_when_available() {
        let rig = load().expect("the fox loads from the repository");
        assert_eq!(rig.model.clips.len(), 3);
        assert_eq!(rig.graph.root_bone, None);
        assert_eq!(rig.scale, MODEL_SCALE);
        assert_eq!(rig.graph.layers.len(), 1, "the fox has no wave layer");
        let chosen = rig::select(false);
        assert_eq!(chosen.model.clips.len(), 3);
    }

    #[test]
    fn the_graph_plays_idle_when_standing_and_moves_when_fast() {
        let rig = load().expect("the fox loads from the repository");
        let mut player = GraphPlayer::new(rig.graph.clone(), &rig.model.skeleton);
        for _ in 0..30 {
            player.update(1.0 / 60.0, &rig.model.skeleton, &rig.model.clips);
        }
        assert_eq!(player.state_name(0), Some("idle"));
        player.params.set_float("speed", 1.0);
        for _ in 0..30 {
            player.update(1.0 / 60.0, &rig.model.skeleton, &rig.model.clips);
        }
        assert_eq!(player.state_name(0), Some("move"));
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let err = load_from(Path::new("no/such/Fox.glb")).unwrap_err();
        assert!(err.contains("Fox.glb"));
    }
}

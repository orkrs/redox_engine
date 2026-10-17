//! The fox: a rigged glTF model (`Fox.glb`, Khronos glTF sample assets) with three clips.
//!
//! * model: PixelMannen, CC0
//! * rigging and animation: tomkranis, CC BY 4.0
//!
//! The importer returns the skeleton, the bind-pose geometry and the clips; the base-colour
//! texture is read from the same file. An animation graph blends Survey (idle) with Walk and Run
//! by the ground speed.

use redox_anim::graph::{AnimGraph, Compare, Condition, LayerDef, Motion, StateDef, TransitionDef};
use redox_anim::{SkinnedModel, gltf_import};

/// Scale from the model's units (about 80 tall) to metres.
pub const MODEL_SCALE: f32 = 0.012;

/// The fox model and its texture.
pub struct FoxAssets {
    pub model: SkinnedModel,
    /// PNG bytes of the base-colour texture.
    pub texture_png: Vec<u8>,
    idle: usize,
    walk: usize,
    run: usize,
}

fn clip_index(model: &SkinnedModel, name: &str) -> Result<usize, String> {
    model
        .clips
        .iter()
        .position(|c| c.name == name)
        .ok_or_else(|| format!("the fox has no clip named {name:?}"))
}

/// Re-encodes the first image of the file as PNG (the renderer decodes encoded images).
fn texture_png(glb: &[u8]) -> Result<Vec<u8>, String> {
    let (_, _, images) = gltf::import_slice(glb).map_err(|e| e.to_string())?;
    let image = images.first().ok_or("the fox file has no texture")?;
    let rgba: Vec<u8> = match image.format {
        gltf::image::Format::R8G8B8 => image
            .pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        gltf::image::Format::R8G8B8A8 => image.pixels.clone(),
        other => return Err(format!("unsupported texture format {other:?}")),
    };
    let buffer = image::RgbaImage::from_raw(image.width, image.height, rgba)
        .ok_or("texture pixels do not match its size")?;
    let mut out = std::io::Cursor::new(Vec::new());
    buffer
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Parses `Fox.glb`.
pub fn load(glb: &[u8]) -> Result<FoxAssets, String> {
    let model = gltf_import::load_glb(glb)?
        .into_iter()
        .next()
        .ok_or("no skinned model in the file")?;
    let idle = clip_index(&model, "Survey")?;
    let walk = clip_index(&model, "Walk")?;
    let run = clip_index(&model, "Run")?;
    Ok(FoxAssets {
        texture_png: texture_png(glb)?,
        model,
        idle,
        walk,
        run,
    })
}

impl FoxAssets {
    /// Idle when still, a walk-to-run blend by `speed` (0..1 of the sprint speed) when moving.
    pub fn graph(&self) -> AnimGraph {
        let speed = |compare, value| Condition::Float {
            param: "speed".into(),
            compare,
            value,
        };
        AnimGraph {
            root_bone: None,
            layers: vec![LayerDef {
                states: vec![
                    StateDef::new("idle", Motion::Clip(self.idle)),
                    StateDef::new(
                        "move",
                        Motion::Blend1D {
                            param: "speed".into(),
                            entries: vec![(0.0, self.walk), (0.4, self.walk), (1.0, self.run)],
                        },
                    ),
                ],
                transitions: vec![
                    TransitionDef {
                        from: Some(0),
                        to: 1,
                        conditions: vec![speed(Compare::Greater, 0.05)],
                        duration: 0.15,
                        exit_time: None,
                    },
                    TransitionDef {
                        from: Some(1),
                        to: 0,
                        conditions: vec![speed(Compare::Less, 0.05)],
                        duration: 0.25,
                        exit_time: None,
                    },
                ],
                entry: 0,
                weight: 1.0,
                mask: None,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use redox_anim::graph::GraphPlayer;

    use super::*;

    fn fox() -> FoxAssets {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/foxrun/Fox.glb");
        load(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn the_fox_loads_with_its_three_clips_and_a_texture() {
        let fox = fox();
        assert_eq!(fox.model.skeleton.len(), 24);
        assert_eq!(fox.model.clips.len(), 3);
        assert!(fox.model.geometry.vertex_count() > 1000);
        fox.model
            .geometry
            .validate(fox.model.skeleton.len())
            .unwrap();
        assert_eq!(&fox.texture_png[1..4], b"PNG");
        let image = image::load_from_memory(&fox.texture_png).unwrap();
        assert_eq!((image.width(), image.height()), (1024, 1024));
    }

    #[test]
    fn the_fox_is_a_sensible_size_in_metres() {
        let fox = fox();
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &fox.model.geometry.positions {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        let height = (max[1] - min[1]) * MODEL_SCALE;
        let length = (max[2] - min[2]) * MODEL_SCALE;
        assert!((0.6..1.4).contains(&height), "height {height}");
        assert!((1.2..2.4).contains(&length), "length {length}");
    }

    #[test]
    fn the_graph_switches_between_idle_and_moving() {
        let fox = fox();
        let mut player = GraphPlayer::new(fox.graph(), &fox.model.skeleton);
        for _ in 0..30 {
            player.update(1.0 / 60.0, &fox.model.skeleton, &fox.model.clips);
        }
        assert_eq!(player.state_name(0), Some("idle"));
        player.params.set_float("speed", 0.8);
        for _ in 0..30 {
            player.update(1.0 / 60.0, &fox.model.skeleton, &fox.model.clips);
        }
        assert_eq!(player.state_name(0), Some("move"));
        player.params.set_float("speed", 0.0);
        for _ in 0..60 {
            player.update(1.0 / 60.0, &fox.model.skeleton, &fox.model.clips);
        }
        assert_eq!(player.state_name(0), Some("idle"));
    }

    #[test]
    fn a_missing_clip_is_reported() {
        let mut fox = fox();
        fox.model.clips.retain(|c| c.name != "Run");
        let err = clip_index(&fox.model, "Run").unwrap_err();
        assert!(err.contains("Run"));
    }
}

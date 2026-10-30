//! Loads the `.hlod` proxies written by `redox_cook bake-hlod` next to a binary
//! world (`world.bin`) and activates HLOD for that world.
//!
//! HLOD is active only when the folder of the world holds at least one readable
//! proxy and `actor_kinds.ron`. Otherwise [`load_hlod_scene`] returns `None` and
//! the frame is left unchanged.

use std::fs;
use std::path::{Path, PathBuf};

use redox_math::Vec3;
use redox_render::hlod::{HlodConfig, HlodProxy};
use redox_render::hlod_gpu::ProxyGpu;
use redox_scene::world_bin::BinWorld;
use redox_scene::{ActorKindTable, PartitionCell};

use crate::app::AppBuilder;
use crate::dispatcher::Stage;
use crate::hlod_system::{HlodRuntime, HlodSettings, hlod_frame_system};
use crate::plugin::Plugin;

/// Name of the actor kind table read next to the world file.
pub const KINDS_FILE_NAME: &str = "actor_kinds.ron";

/// What HLOD needs from a loaded scene: the cross-fade runtime and the kind table
/// that says which actors it hides.
pub struct HlodScene {
    /// Cell states and uploaded proxies.
    pub runtime: HlodRuntime,
    /// Kind table of the scene (`hlod` flags select the hidden actors).
    pub kinds: ActorKindTable,
}

/// File name of the proxy of `cell`, as written by `bake-hlod`: `<x>_<y>.hlod`.
pub fn proxy_file_name(cell: PartitionCell) -> String {
    format!("{}_{}.hlod", cell.x, cell.y)
}

/// The cells of `cells` that have a proxy file in `dir`, in input order.
pub fn existing_proxies(dir: &Path, cells: &[PartitionCell]) -> Vec<(PartitionCell, PathBuf)> {
    cells
        .iter()
        .map(|cell| (*cell, dir.join(proxy_file_name(*cell))))
        .filter(|(_, path)| path.is_file())
        .collect()
}

/// Decodes one proxy file. The cell stored in the file must be `expected`.
///
/// # Errors
/// A message naming the file when it cannot be read or decoded, or stores another cell.
pub fn read_proxy_file(path: &Path, expected: PartitionCell) -> Result<HlodProxy, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (cell, proxy) =
        HlodProxy::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if cell != (expected.x, expected.y) {
        return Err(format!(
            "{}: stores cell {}_{}, expected {}_{}",
            path.display(),
            cell.0,
            cell.1,
            expected.x,
            expected.y
        ));
    }
    Ok(proxy)
}

/// Loads the proxies and the kind table that belong to `world_bin`.
///
/// `upload` turns a decoded proxy into GPU slots (normally
/// `|proxy| upload_proxy(renderer, proxy)`). A proxy that cannot be read is
/// skipped with a warning, so its cell keeps its detail objects.
///
/// Returns `Ok(None)` when no proxy can be used or `actor_kinds.ron` is missing.
///
/// # Errors
/// A message when the world file or the kind table exists but cannot be parsed.
pub fn load_hlod_scene(
    world_bin: &Path,
    config: HlodConfig,
    mut upload: impl FnMut(&HlodProxy) -> ProxyGpu,
) -> Result<Option<HlodScene>, String> {
    let dir = world_bin.parent().unwrap_or_else(|| Path::new("."));
    let bin = BinWorld::open(world_bin).map_err(|e| format!("{}: {e}", world_bin.display()))?;
    let cells: Vec<PartitionCell> = bin.cells().collect();
    let proxies = existing_proxies(dir, &cells);
    if proxies.is_empty() {
        return Ok(None);
    }
    let kinds_path = dir.join(KINDS_FILE_NAME);
    if !kinds_path.is_file() {
        log::warn!(
            "HLOD proxies found in {} but no {KINDS_FILE_NAME}; HLOD stays off",
            dir.display()
        );
        return Ok(None);
    }
    let kinds_text = fs::read_to_string(&kinds_path)
        .map_err(|e| format!("{}: {e}", kinds_path.display()))?;
    let kinds = ActorKindTable::from_ron(&kinds_text)
        .map_err(|e| format!("{}: {e}", kinds_path.display()))?;

    let mut uploaded = Vec::new();
    for (cell, path) in proxies {
        match read_proxy_file(&path, cell) {
            Ok(proxy) => uploaded.push((cell, upload(&proxy))),
            Err(message) => log::warn!("HLOD skips cell {}_{}: {message}", cell.x, cell.y),
        }
    }
    if uploaded.is_empty() {
        return Ok(None);
    }

    let manifest = bin.manifest();
    let mut runtime = HlodRuntime::new(
        config,
        manifest.cell_size,
        Vec3::from_array(manifest.origin),
        uploaded.iter().map(|(cell, _)| *cell),
    );
    for (cell, gpu) in uploaded {
        runtime.register_proxy(cell, gpu);
    }
    Ok(Some(HlodScene { runtime, kinds }))
}

/// Loads the HLOD of a scene at start-up and runs its frame step.
///
/// The world file is `world_bin`; its `.hlod` files and `actor_kinds.ron` are read
/// from the same folder. Without them nothing is inserted and the frame does not change.
/// The frame step is a system on [`Stage::Render`]: it reads the objects that
/// extraction put in `FrameRenderObjects` earlier in the frame, so the draw system
/// must be added after this plugin.
// ASSUMPTION: systems of one stage run in registration order.
pub struct HlodPlugin {
    /// Path of the scene's `world.bin`.
    pub world_bin: PathBuf,
    /// Switch distance, hysteresis and fade time of the cell cross-fade.
    pub config: HlodConfig,
    /// Initial value of [`HlodSettings::enabled`]; the step still does nothing without proxies.
    pub enabled: bool,
}

impl Plugin for HlodPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let world_bin = self.world_bin.clone();
        let config = self.config;
        let enabled = self.enabled;
        app.on_render_setup(move |world, render| {
            world.insert_resource(HlodSettings { enabled });
            let scene = load_hlod_scene(&world_bin, config, |proxy| {
                redox_render::hlod_gpu::upload_proxy(render, proxy)
            });
            match scene {
                Ok(Some(scene)) => {
                    if world.get_resource::<ActorKindTable>().is_none() {
                        world.insert_resource(scene.kinds);
                    }
                    world.insert_resource(scene.runtime);
                }
                Ok(None) => {}
                Err(message) => log::warn!("HLOD stays off: {message}"),
            }
        });
        app.add_system_mut(Stage::Render, |world, _| hlod_frame_system(world));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_render::hlod::{HlodInput, HlodParams, build_proxy};
    use redox_render::hlod_gpu::ProxyGpu;
    use redox_math::{Aabb, Mat4};
    use redox_scene::world_bin::write_world_bin;
    use redox_scene::{ActorRecord, WorldManifest};

    const KINDS: &str = r#"(kinds: { "crate": (mesh: "crate.glb", color: (0.5, 0.25, 0.0)) })"#;

    /// A fresh folder under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "redox_hlod_load_{tag}_{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn two_cells() -> [PartitionCell; 2] {
        [PartitionCell { x: 0, y: 0 }, PartitionCell { x: 1, y: 0 }]
    }

    /// Writes a world.bin with two cells, one actor each.
    fn write_world(dir: &Path) -> PathBuf {
        let world = dir.join("world.bin");
        let actor = ActorRecord {
            id: "a".into(),
            kind: "crate".into(),
            position: [1.0, 0.0, 1.0],
            yaw: 0.0,
            tags: Vec::new(),
        };
        let cells = two_cells().map(|cell| (cell, vec![actor.clone()]));
        write_world_bin(&world, WorldManifest::new("test", 50.0, 1), &cells).unwrap();
        world
    }

    fn triangle_proxy() -> HlodProxy {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let indices = [0_u32, 1, 2];
        let input = HlodInput {
            positions: &positions,
            indices: &indices,
            transform: Mat4::IDENTITY,
            color: [1.0, 0.0, 0.0],
            vertex_colors: None,
        };
        let params = HlodParams {
            target_triangles: 10,
            max_error: 0.01,
            weld_epsilon: 1e-4,
        };
        build_proxy(&[input], &params).unwrap()
    }

    fn dummy_gpu() -> ProxyGpu {
        ProxyGpu {
            mesh_slot: 0,
            material_slot: 0,
            bounds: Aabb::empty(),
        }
    }

    #[test]
    fn a_proxy_file_is_named_by_its_cell() {
        assert_eq!(proxy_file_name(PartitionCell { x: 3, y: -4 }), "3_-4.hlod");
    }

    #[test]
    fn only_the_cells_with_a_file_are_listed() {
        let dir = TempDir::new("listing");
        fs::write(dir.0.join("1_0.hlod"), b"x").unwrap();
        let found = existing_proxies(&dir.0, &two_cells());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, PartitionCell { x: 1, y: 0 });
    }

    #[test]
    fn a_file_for_another_cell_is_rejected() {
        let dir = TempDir::new("wrong_cell");
        let path = dir.0.join("0_0.hlod");
        fs::write(&path, triangle_proxy().to_bytes((5, 5))).unwrap();
        let result = read_proxy_file(&path, PartitionCell { x: 0, y: 0 });
        assert!(result.is_err());
    }

    #[test]
    fn a_proxy_file_round_trips_through_the_loader() {
        let dir = TempDir::new("roundtrip");
        let path = dir.0.join("0_0.hlod");
        let proxy = triangle_proxy();
        fs::write(&path, proxy.to_bytes((0, 0))).unwrap();
        let loaded = read_proxy_file(&path, PartitionCell { x: 0, y: 0 }).unwrap();
        assert_eq!(loaded, proxy);
    }

    #[test]
    fn a_scene_without_proxies_is_not_activated() {
        let dir = TempDir::new("no_proxies");
        let world = write_world(&dir.0);
        fs::write(dir.0.join(KINDS_FILE_NAME), KINDS).unwrap();
        let scene = load_hlod_scene(&world, HlodConfig::default(), |_| dummy_gpu()).unwrap();
        assert!(scene.is_none());
    }

    #[test]
    fn proxies_without_a_kind_table_do_not_activate_hlod() {
        let dir = TempDir::new("no_kinds");
        let world = write_world(&dir.0);
        fs::write(dir.0.join("0_0.hlod"), triangle_proxy().to_bytes((0, 0))).unwrap();
        let scene = load_hlod_scene(&world, HlodConfig::default(), |_| dummy_gpu()).unwrap();
        assert!(scene.is_none());
    }

    #[test]
    fn present_proxies_are_registered_and_damaged_ones_skipped() {
        let dir = TempDir::new("present");
        let world = write_world(&dir.0);
        fs::write(dir.0.join(KINDS_FILE_NAME), KINDS).unwrap();
        fs::write(dir.0.join("0_0.hlod"), triangle_proxy().to_bytes((0, 0))).unwrap();
        fs::write(dir.0.join("1_0.hlod"), b"RHLD-damaged").unwrap();
        let scene = load_hlod_scene(&world, HlodConfig::default(), |_| dummy_gpu())
            .unwrap()
            .unwrap();
        assert!(scene.runtime.has_proxy(PartitionCell { x: 0, y: 0 }));
        assert!(!scene.runtime.has_proxy(PartitionCell { x: 1, y: 0 }));
        assert!(scene.kinds.is_hlod("crate"));
    }

    #[test]
    fn a_missing_world_file_is_an_error() {
        let dir = TempDir::new("missing_world");
        let result = load_hlod_scene(&dir.0.join("nope.bin"), HlodConfig::default(), |_| {
            dummy_gpu()
        });
        assert!(result.is_err());
    }
}

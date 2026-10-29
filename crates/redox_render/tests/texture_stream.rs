//! Texture streaming on a real GPU: progressive upload, promotion, demotion, budget, hysteresis
//! and the scene-based choice of mip.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use redox_asset::testing::Ktx2Builder;
use redox_asset::{AssetManager, Handle, Ktx2Texture, Priority, StreamedTexture, TextureFormat};
use redox_ecs::World;
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::asset_types::{MaterialData, MeshData};
use redox_render::camera::Camera;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::resource::texture_stream::{StreamConfig, TextureStreamer};
use redox_render::systems::RenderObject;

const SIZE: u32 = 96;
const EDGE: u32 = 256;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("redox_gpu_stream_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Rig {
    renderer: Renderer,
    assets: AssetManager,
    world: World,
    streamer: TextureStreamer,
    builder: Ktx2Builder,
    _dir: TempDir,
}

fn config() -> StreamConfig {
    StreamConfig {
        floor_edge: 64,
        demote_after_frames: 0,
        ..Default::default()
    }
}

fn rig(tag: &str, config: StreamConfig, files: &[&str]) -> Option<Rig> {
    let adapter = common::adapter()?;
    let renderer = common::renderer(&adapter, SIZE)?;
    let dir = TempDir::new(tag);
    let builder = Ktx2Builder::new(TextureFormat::Rgba8Unorm, EDGE, EDGE);
    for name in files {
        std::fs::write(dir.0.join(name), builder.build()).unwrap();
    }
    Some(Rig {
        renderer,
        assets: AssetManager::with_workers(&dir.0, 2),
        world: World::new(),
        streamer: TextureStreamer::new(config),
        builder,
        _dir: dir,
    })
}

impl Rig {
    fn step(&mut self) {
        self.assets.update(&mut self.world);
        self.streamer.update(&mut self.renderer, &mut self.assets);
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }

    fn run_until(&mut self, what: &str, done: impl Fn(&Rig) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            self.step();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The GPU contents of a streamed texture, from its resident base down.
    fn read(&self, handle: Handle<StreamedTexture>) -> Ktx2Texture {
        let slot = self.streamer.slot_of(handle).unwrap();
        let texture = &self.renderer.registry.texture_at(slot).unwrap().texture;
        read_texture_ktx2(
            &self.renderer.device,
            &self.renderer.queue,
            &texture.texture,
            false,
        )
        .unwrap()
    }

    fn assert_matches_source(&self, handle: Handle<StreamedTexture>) {
        let base = self.streamer.resident_base(handle).unwrap() as usize;
        let on_gpu = self.read(handle);
        let source = self.builder.raw_levels();
        assert_eq!(
            on_gpu.levels.len(),
            source.len() - base,
            "resident mip count"
        );
        for (i, level) in on_gpu.levels.iter().enumerate() {
            assert!(
                level == &source[base + i],
                "mip {} differs from the file",
                base + i
            );
        }
    }

    fn base(&self, handle: Handle<StreamedTexture>) -> Option<u32> {
        self.streamer.resident_base(handle)
    }
}

fn open(rig: &mut Rig, name: &str) -> Handle<StreamedTexture> {
    rig.streamer
        .add(&mut rig.assets, name, Priority::HIGH)
        .unwrap()
}

#[test]
fn small_mips_arrive_first_then_the_texture_sharpens_to_full_size() {
    let Some(mut rig) = rig("progressive", config(), &["t.ktx2"]) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let tex = open(&mut rig, "t.ktx2");
    rig.run_until("the texture to exist", |r| r.base(tex).is_some());
    // 256 px with a 64 px floor: mips 2.. are resident first.
    assert_eq!(rig.base(tex), Some(2));
    rig.run_until("full size", |r| r.base(tex) == Some(0));
    rig.assert_matches_source(tex);
    assert!(rig.streamer.stats().rebuilds >= 1);
}

#[test]
fn asking_for_less_detail_demotes_and_asking_again_restores_identical_data() {
    let Some(mut rig) = rig("demote", config(), &["t.ktx2"]) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let tex = open(&mut rig, "t.ktx2");
    rig.run_until("full size", |r| r.base(tex) == Some(0));
    let full_bytes = rig.streamer.stats().resident_bytes;

    for _ in 0..3 {
        rig.streamer.set_desired_mip(tex, 4);
        rig.step();
    }
    assert_eq!(rig.base(tex), Some(2), "demoted to the resident floor");
    assert!(rig.streamer.stats().resident_bytes < full_bytes / 8);
    rig.assert_matches_source(tex);

    // A mid-way request promotes only that far.
    for _ in 0..400 {
        rig.streamer.set_desired_mip(tex, 1);
        rig.step();
        if rig.base(tex) == Some(1) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(rig.base(tex), Some(1));
    rig.assert_matches_source(tex);

    for _ in 0..400 {
        rig.streamer.set_desired_mip(tex, 0);
        rig.step();
        if rig.base(tex) == Some(0) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(rig.base(tex), Some(0));
    rig.assert_matches_source(tex);
}

#[test]
fn a_sharp_mip_survives_a_short_gap_in_demand() {
    let cfg = StreamConfig {
        demote_after_frames: 40,
        ..config()
    };
    let Some(mut rig) = rig("hysteresis", cfg, &["t.ktx2"]) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let tex = open(&mut rig, "t.ktx2");
    rig.run_until("full size", |r| r.base(tex) == Some(0));
    for _ in 0..10 {
        rig.streamer.set_desired_mip(tex, 4);
        rig.step();
    }
    assert_eq!(rig.base(tex), Some(0), "still sharp inside the hold time");
    for _ in 0..60 {
        rig.streamer.set_desired_mip(tex, 4);
        rig.step();
    }
    assert_eq!(rig.base(tex), Some(2), "demoted after the hold time");
}

#[test]
fn the_memory_budget_demotes_the_least_recently_needed_texture() {
    let cfg = StreamConfig {
        demote_after_frames: 1000,
        ..config()
    };
    let Some(mut rig) = rig("budget", cfg, &["a.ktx2", "b.ktx2"]) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let a = open(&mut rig, "a.ktx2");
    let b = open(&mut rig, "b.ktx2");
    rig.run_until("both at full size", |r| {
        r.base(a) == Some(0) && r.base(b) == Some(0)
    });
    let one = rig.streamer.stats().resident_bytes / 2;

    // Room for one full texture and a bit: the one that stays wanted keeps its detail.
    rig.streamer.set_texture_budget(one + one / 4);
    for _ in 0..5 {
        rig.streamer.set_desired_mip(a, 0);
        rig.streamer.set_desired_mip(b, 5);
        rig.step();
    }
    assert_eq!(rig.base(a), Some(0), "the used texture keeps its detail");
    assert!(
        rig.base(b).unwrap() > 0,
        "the idle texture gave memory back"
    );
    assert!(rig.streamer.stats().resident_bytes <= one + one / 4);
    rig.assert_matches_source(a);
    rig.assert_matches_source(b);
}

#[test]
fn on_screen_size_picks_the_mip_and_materials_follow_the_new_texture() {
    let Some(mut rig) = rig("scene", config(), &["t.ktx2"]) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let tex = open(&mut rig, "t.ktx2");
    rig.run_until("the texture to exist", |r| r.base(tex).is_some());
    let slot = rig.streamer.slot_of(tex).unwrap();

    let ktx_handle: Handle<Ktx2Texture> = Handle::new(tex.id());
    let material = rig
        .assets
        .insert(MaterialData::solid(Vec3::ONE).with_albedo_ktx2(ktx_handle));
    let mesh: Handle<MeshData> = rig.assets.insert(create_cube());
    let mesh_index = rig
        .renderer
        .add_mesh_from_asset(mesh, rig.assets.get(mesh).unwrap())
        .unwrap();
    let material_index = rig
        .renderer
        .add_material_from_asset(material, rig.assets.get(material).unwrap())
        .unwrap();
    assert_eq!(
        rig.renderer
            .registry
            .legacy_material(material_index)
            .unwrap()
            .texture_index,
        Some(slot)
    );

    let camera = Camera::new(0.9, 1.0, 0.1, 500.0);
    let object_at = |z: f32| RenderObject {
        id: 0,
        shadow_class: redox_render::shadow::ShadowClass::Auto,
        model_matrix: Mat4::from_translation(Vec3::new(0.0, 0.0, -z)),
        color: [1.0; 4],
        mesh_index,
        material_index,
        bounds: Aabb::from_center_size(Vec3::new(0.0, 0.0, -z), Vec3::splat(0.5)),
    };

    // Far away: a few pixels on screen, the floor is plenty.
    rig.renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    for _ in 0..30 {
        rig.streamer
            .update_from_scene(&rig.renderer, &[object_at(300.0)]);
        rig.step();
    }
    assert_eq!(rig.base(tex), Some(2), "far object stays at the floor");

    // Close: the object covers the view, so every texel counts.
    let before = rig.renderer.registry.legacy_material_bind_groups().len();
    let deadline = Instant::now() + Duration::from_secs(20);
    while rig.base(tex) != Some(0) {
        assert!(
            Instant::now() < deadline,
            "close object never got full detail"
        );
        rig.streamer
            .update_from_scene(&rig.renderer, &[object_at(1.6)]);
        rig.step();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        rig.renderer.registry.legacy_material_bind_groups().len(),
        before
    );
    rig.assert_matches_source(tex);
}

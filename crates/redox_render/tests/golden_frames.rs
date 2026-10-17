//! Golden frames for the texture work: lighting from baked KTX2 IBL, and a streamed texture after
//! it has converged. Skipped without a GPU.
//!
//! * IBL: the frame lit by IBL that went through KTX2 files must equal the frame lit by the
//!   generated maps, and match the stored image `tests/golden/ibl_ktx2.png`
//!   (`REDOX_UPDATE_GOLDEN=1` rewrites it after an intentional change).
//! * Streaming: once the streamer has converged, the frame must equal the frame of the same
//!   texture loaded whole; before that it must differ (the texture really was blurry).

#![allow(clippy::unwrap_used)]

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use redox_asset::testing::Ktx2Builder;
use redox_asset::{AssetLoader, AssetManager, Handle, Ktx2Loader, Ktx2Texture, Priority, TextureFormat};
use redox_ecs::World;
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::asset_types::{MaterialData, MeshData};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::{IBLProcessor, IBLResource, read_texture_ktx2};
use redox_render::resource::texture::Texture;
use redox_render::resource::texture_stream::{StreamConfig, TextureStreamer};
use redox_render::systems::{RenderObject, sync_ktx2_textures_to_render};

const SIZE: u32 = 96;
/// Mean absolute channel error (0..255) under which two frames count as equal.
const TOLERANCE: f64 = 1.0;

struct Scene {
    camera: Camera,
    lights: LightScene,
    object: RenderObject,
}

fn scene(renderer: &mut Renderer, mesh: Handle<MeshData>, material: usize, assets: &AssetManager) -> Scene {
    let mesh_index = renderer
        .add_mesh_from_asset(mesh, assets.get(mesh).unwrap())
        .unwrap();
    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 2.2), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(Vec3::new(0.3, -0.5, -1.0), Vec3::ONE, 2.0));
    lights.ambient = Vec3::splat(0.4);
    Scene {
        camera,
        lights,
        object: RenderObject {
            model_matrix: Mat4::from_quat(Quat::from_rotation_y(0.6)),
            color: [1.0; 4],
            mesh_index,
            material_index: material,
            bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::splat(0.5)),
        },
    }
}

/// Draws a few frames (a whole TAA jitter cycle, so frames of different draws line up) and returns the last as RGBA bytes.
fn draw(renderer: &mut Renderer, scene: &Scene, material: usize) -> Vec<u8> {
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("golden_target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let objects = [RenderObject {
        material_index: material,
        ..scene.object.clone()
    }];
    for _ in 0..16 {
        let fv = FrameView::new(&scene.camera, &objects, &scene.lights);
        renderer.render_frame_view(&fv, &view).expect("frame");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
    }
    read_texture_ktx2(&renderer.device, &renderer.queue, &target, false)
        .unwrap()
        .levels
        .swap_remove(0)
}

fn mean_error(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (*x as f64 - *y as f64).abs())
        .sum::<f64>()
        / a.len() as f64
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

fn check_against_file(name: &str, frame: &[u8]) {
    let path = golden_path(name);
    if std::env::var_os("REDOX_UPDATE_GOLDEN").is_some() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::save_buffer(&path, frame, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        eprintln!("golden: stored {}", path.display());
        return;
    }
    let stored = image::open(&path).unwrap().to_rgba8();
    let error = mean_error(stored.as_raw(), frame);
    assert!(error < TOLERANCE, "{name}: mean error {error:.3} against the stored frame");
}

fn shiny(assets: &mut AssetManager) -> Handle<MaterialData> {
    let mut data = MaterialData::solid(Vec3::new(0.9, 0.55, 0.3));
    data.metallic = 0.9;
    data.roughness = 0.3;
    assets.insert(data)
}

#[test]
fn ibl_baked_to_ktx2_lights_a_frame_like_the_generated_maps() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    let mut assets = AssetManager::new(".");
    let mesh: Handle<MeshData> = assets.insert(create_cube());
    let material = shiny(&mut assets);
    let material_index = renderer
        .add_material_from_asset(material, assets.get(material).unwrap())
        .unwrap();
    let scene = scene(&mut renderer, mesh, material_index, &assets);

    // A deterministic sky: bright warm horizon, cool zenith.
    let sky = image::RgbaImage::from_fn(64, 32, |x, y| {
        image::Rgba([(220 - y * 4) as u8, (150 + y * 2) as u8, (90 + x * 2) as u8, 255])
    });
    let equirect = Texture::from_image(
        &renderer.device,
        &renderer.queue,
        &image::DynamicImage::ImageRgba8(sky),
        "sky",
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    let processor = IBLProcessor::new(&renderer.device);
    let generated = IBLResource::from_equirect(&renderer.device, &renderer.queue, &processor, &equirect);
    let baked = generated.export_ktx2(&renderer.device, &renderer.queue).unwrap();

    renderer.set_ibl_resource(generated);
    let from_generated = draw(&mut renderer, &scene, material_index);

    let upload = |tex: &Ktx2Texture, label: &str| {
        let loaded = Ktx2Loader::default().load(&tex.to_ktx2_bytes(true)).unwrap();
        Texture::from_ktx2(&renderer.device, &renderer.queue, &loaded, label).unwrap()
    };
    let from_files = IBLResource::from_ktx2(
        upload(&baked.environment, "env"),
        upload(&baked.irradiance, "irr"),
        upload(&baked.prefiltered, "pre"),
        upload(&baked.brdf_lut, "lut"),
    )
    .unwrap();
    renderer.set_ibl_resource(from_files);
    let from_ktx2 = draw(&mut renderer, &scene, material_index);

    let error = mean_error(&from_generated, &from_ktx2);
    assert!(error < TOLERANCE, "KTX2 IBL differs from the generated one: {error:.3}");
    check_against_file("ibl_ktx2.png", &from_ktx2);
}

fn pump(renderer: &mut Renderer, assets: &mut AssetManager, world: &mut World, streamer: &mut TextureStreamer) {
    assets.update(world);
    streamer.update(renderer, assets);
    let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
}

#[test]
fn a_streamed_texture_looks_like_the_whole_one_once_it_has_converged() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("redox_golden_stream_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = Ktx2Builder::new(TextureFormat::Rgba8Srgb, 256, 256).build();
    std::fs::write(dir.join("t.ktx2"), &file).unwrap();

    let mut assets = AssetManager::with_workers(&dir, 2);
    let mut world = World::new();
    let mut streamer = TextureStreamer::new(StreamConfig {
        floor_edge: 32,
        ..Default::default()
    });

    // The same texture, loaded whole, as the reference.
    let whole = assets.insert(Ktx2Loader::default().load(&file).unwrap());
    sync_ktx2_textures_to_render(&mut renderer, &assets, &[whole]);
    let streamed = streamer.add(&mut assets, "t.ktx2", Priority::HIGH).unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    while streamer.slot_of(streamed).is_none() {
        assert!(Instant::now() < deadline, "texture never opened");
        pump(&mut renderer, &mut assets, &mut world, &mut streamer);
    }
    let mesh: Handle<MeshData> = assets.insert(create_cube());
    let whole_material = assets.insert(MaterialData::solid(Vec3::ONE).with_albedo_ktx2(whole));
    let streamed_material = assets.insert(
        MaterialData::solid(Vec3::ONE).with_albedo_ktx2(Handle::<Ktx2Texture>::new(streamed.id())),
    );
    let whole_index = renderer
        .add_material_from_asset(whole_material, assets.get(whole_material).unwrap())
        .unwrap();
    let streamed_index = renderer
        .add_material_from_asset(streamed_material, assets.get(streamed_material).unwrap())
        .unwrap();
    let scene = scene(&mut renderer, mesh, whole_index, &assets);

    let reference = draw(&mut renderer, &scene, whole_index);
    let early = draw(&mut renderer, &scene, streamed_index);
    assert!(
        streamer.resident_base(streamed).unwrap() > 0 || mean_error(&reference, &early) < TOLERANCE,
        "a texture that is not converged has to be blurrier"
    );

    while streamer.resident_base(streamed) != Some(0) {
        assert!(Instant::now() < deadline, "streaming never converged");
        pump(&mut renderer, &mut assets, &mut world, &mut streamer);
        std::thread::sleep(Duration::from_millis(1));
    }
    let converged = draw(&mut renderer, &scene, streamed_index);
    let error = mean_error(&reference, &converged);
    assert!(error < TOLERANCE, "converged streamed frame differs from the whole texture: {error:.3}");
    let _ = std::fs::remove_dir_all(&dir);
}

//! GI cost benchmark (GI-01 step 6). Ignored by default; run with
//! `cargo test -p redox_render --test gi_bench -- --ignored --nocapture`.
//!
//! Scene: the Cornell room plus 3000 small static boxes on the floor, all as the static SDF volume.
//! For each quality the GPU time of the passes whose label starts with `gi` (trace and update) is
//! averaged over the frames after the volume is ready. Prints the numbers; the coordinator enters
//! them in docs/BENCHMARKS.md. Target: Medium at or below 2.5 ms on 1080p (GI-01 gate A7).
//! If Medium is above the target, the number is reported, not optimised here.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use redox_asset::{AssetManager, Handle};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::asset_types::{MaterialData, MeshData};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::gi::probes::GiQuality;
use redox_render::gi::sdf_ref::cornell_box;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::mesh::primitive::create_cube;
use redox_render::sdf::global::SdfBox;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

/// Square frame size; 1080 is the spec's reference resolution.
const SIZE: u32 = 1080;
/// Frames drawn per quality.
const FRAMES: u32 = 150;
/// First frame whose timings count: the volume rebuild and the atlas warm-up come before it.
const SAMPLE_FROM: u32 = 60;
/// Small static boxes on the floor, in addition to the Cornell room.
const FLOOR_BOXES: u32 = 3000;
/// Qualities measured, in the order the report prints them.
const QUALITIES: [(&str, GiQuality); 3] = [
    ("High", GiQuality::High),
    ("Medium", GiQuality::Medium),
    ("Low", GiQuality::Low),
];

/// `FLOOR_BOXES` boxes of 5 cm on a 6 cm grid over the floor of the 4 m room.
fn floor_boxes(count: u32) -> Vec<SdfBox> {
    (0..count)
        .map(|i| {
            let x = -1.8 + (i % 60) as f32 * 0.06;
            let z = -1.8 + (i / 60) as f32 * 0.06;
            SdfBox {
                min: Vec3::new(x, 0.0, z),
                max: Vec3::new(x + 0.05, 0.05, z + 0.05),
                albedo: Vec3::splat(0.6),
                emissive: Vec3::ZERO,
            }
        })
        .collect()
}

/// One unit cube per box, scaled and placed to cover it, all with one grey material.
fn scene_objects(
    renderer: &mut Renderer,
    assets: &mut AssetManager,
    boxes: &[SdfBox],
) -> Vec<RenderObject> {
    let mesh: Handle<MeshData> = assets.insert(create_cube());
    let mesh_index = renderer
        .add_mesh_from_asset(mesh, assets.get(mesh).unwrap())
        .unwrap();
    let material = assets.insert(MaterialData::solid(Vec3::splat(0.6)));
    let material_index = renderer
        .add_material_from_asset(material, assets.get(material).unwrap())
        .unwrap();
    boxes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let center = (b.min + b.max) * 0.5;
            let size = b.max - b.min;
            RenderObject {
                id: i as u32,
                shadow_class: ShadowClass::Auto,
                deforms: false,
                pose_version: 0,
                model_matrix: Mat4::from_translation(center) * Mat4::from_scale(size),
                color: [1.0; 4],
                mesh_index,
                material_index,
                bounds: Aabb {
                    min: b.min,
                    max: b.max,
                },
            }
        })
        .collect()
}

/// Sum of the GI pass timings of the last frame read back, in milliseconds.
fn gi_ms(renderer: &Renderer) -> f32 {
    renderer
        .gpu_profiler
        .timings()
        .iter()
        .filter(|t| t.label.starts_with("gi"))
        .map(|t| t.ms)
        .sum()
}

#[test]
#[ignore = "GI-01 step 6 benchmark: run with --ignored --nocapture"]
fn gi_cost_by_quality() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    if !renderer.gpu_profiler.is_enabled() {
        eprintln!("GPU timestamps are not supported by this adapter; nothing to measure");
        return;
    }
    let mut boxes = cornell_box().boxes;
    boxes.extend(floor_boxes(FLOOR_BOXES));
    renderer.set_gi_static_geometry(&boxes);
    let mut assets = AssetManager::new(".");
    let objects = scene_objects(&mut renderer, &mut assets, &boxes);

    // Indirect light only: the probes are the whole lighting.
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::ONE,
        0.0,
    ));
    lights.ambient = Vec3::ZERO;
    let camera = Camera::new(1.2, 1.0, 0.05, 30.0);
    renderer.camera_uniform.update(
        &camera,
        Vec3::new(0.0, 1.0, 0.0),
        Quat::from_rotation_x(-0.6),
    );
    renderer.update_camera_buffer();
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gi_bench_target"),
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

    for (name, quality) in QUALITIES {
        renderer.set_gi_quality(quality);
        let mut sum = 0.0f32;
        let mut count = 0u32;
        for frame in 0..FRAMES {
            let fv = FrameView::new(&camera, &objects, &lights);
            renderer.render_frame_view(&fv, &view).expect("frame");
            let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
            if frame >= SAMPLE_FROM {
                sum += gi_ms(&renderer);
                count += 1;
            }
            // The volume rebuild runs on a worker thread; give it time to finish.
            std::thread::sleep(Duration::from_millis(5));
        }
        let mean = sum / count.max(1) as f32;
        println!("gi_bench quality={name} gi_passes_ms_mean={mean:.3} frames={count} scene=cornell+{FLOOR_BOXES}boxes size={SIZE}");
    }
}

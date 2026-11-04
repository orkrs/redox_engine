//! FOL-01 step 2: CPU cost of a frame as the object count grows (1 000 to 10 000).
//! Ignored by default; run with `cargo test -p redox_render --test object_count_bench -- --ignored --nocapture`.
//! Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::Instant;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 512;
const COUNTS: [usize; 4] = [1_000, 2_500, 5_000, 10_000];
const MATERIALS: usize = 4;
const WARMUP_FRAMES: usize = 5;
const TIMED_FRAMES: usize = 30;

/// Small deterministic generator (no extra dependency): values in [0, 1).
fn unit(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    (*state >> 40) as f32 / (1u64 << 24) as f32
}

/// Objects spread over the camera's view: a box in front of the camera.
fn scatter(count: usize, mesh: usize, sorted: bool) -> Vec<RenderObject> {
    let mut seed = 0x5eed_u64;
    let mut objects: Vec<RenderObject> = (0..count)
        .map(|i| {
            let x = (unit(&mut seed) - 0.5) * 30.0;
            let y = (unit(&mut seed) - 0.5) * 30.0;
            let z = -10.0 - unit(&mut seed) * 40.0;
            let center = Vec3::new(x, y, z);
            let scale = Vec3::splat(0.3);
            RenderObject {
                id: i as u32,
                shadow_class: ShadowClass::Auto,
                deforms: false,
                pose_version: 0,
                model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
                color: [1.0; 4],
                mesh_index: mesh,
                material_index: i % MATERIALS,
                bounds: Aabb::from_center_size(center, scale),
            }
        })
        .collect();
    if sorted {
        // Already in (mesh, material) order: the draw order is then the identity.
        objects.sort_by_key(|o| (o.mesh_index, o.material_index));
    }
    objects
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture"]
fn object_count_cpu_cost() {
    let Some(adapter) = common::adapter() else {
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    let mesh = renderer.upload_mesh(&create_cube());
    let materials: Vec<usize> = (0..MATERIALS)
        .map(|i| {
            let t = i as f32 / MATERIALS as f32;
            renderer.add_material(Material::solid(Vec3::new(t, 1.0 - t, 0.5)))
        })
        .collect();
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("object_count_target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.4);

    println!("| Объекты | Порядок | Кадр CPU, мс | prepare, мс | record, мс | submit, мс |");
    println!("|---|---|---|---|---|---|");
    for sorted in [false, true] {
        for &count in &COUNTS {
            let mut objects = scatter(count, mesh, sorted);
            for o in &mut objects {
                o.material_index = materials[o.material_index];
            }
            let (mut wall, mut prep, mut rec, mut sub) = (0.0f64, 0.0f32, 0.0f32, 0.0f32);
            for frame in 0..WARMUP_FRAMES + TIMED_FRAMES {
                let fv = FrameView::new(&camera, &objects, &lights);
                let t = Instant::now();
                renderer.render_frame_view(&fv, &view).expect("frame");
                let elapsed = t.elapsed().as_secs_f64() * 1000.0;
                if frame >= WARMUP_FRAMES {
                    let cpu = renderer.last_frame_cpu_timings();
                    wall += elapsed;
                    prep += cpu.prepare_ms;
                    rec += cpu.record_ms;
                    sub += cpu.submit_ms;
                }
            }
            let n = TIMED_FRAMES as f32;
            let order = if sorted {
                "отсортирован"
            } else {
                "случайный"
            };
            println!(
                "| {count} | {order} | {:.3} | {:.3} | {:.3} | {:.3} |",
                wall / TIMED_FRAMES as f64,
                prep / n,
                rec / n,
                sub / n
            );
        }
    }
}

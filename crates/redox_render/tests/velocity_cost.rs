//! RND-10 step 1: what the separate object-velocity pass costs on a heavy scene (20 moving meshes of
//! about 5k vertices, 3000 static cubes, 50 alpha-blended objects). Prints a table and a verdict
//! ("merge" / "do not merge"); asserts nothing. Run with
//! `cargo test --release -p redox_render --test velocity_cost -- --ignored --nocapture`.
//! Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::asset_types::BlendMode;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 1024;
const MOVING: usize = 20;
const STATIC: usize = 3000;
const TRANSPARENT: usize = 50;
/// Sphere with 100 segments: (100 + 1) * (50 + 1) = 5151 vertices.
const SPHERE_SEGMENTS: u32 = 100;
const WARMUP: usize = 5;
const FRAMES: usize = 30;
/// RND-10 gate: merge when the GPU saving reaches this many milliseconds ...
const GATE_GPU_MS: f32 = 0.3;
/// ... or when the draw calls drop by this share of all draw calls.
const GATE_DRAWS_PERCENT: f32 = 15.0;

fn object(mesh: usize, material: usize, center: Vec3, scale: Vec3) -> RenderObject {
    RenderObject {
        id: 0,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(center, scale),
    }
}

/// Slots of the meshes and materials used by the scene.
struct Slots {
    sphere: usize,
    cube: usize,
    solid: usize,
    moving: usize,
    glass: usize,
}

/// The scene at frame `step`. The list order is the same in every frame, so objects are matched
/// by index and the moving spheres produce object motion.
fn scene(slots: &Slots, step: usize) -> Vec<RenderObject> {
    let drift = step as f32 * 0.05;
    let mut objects = Vec::with_capacity(MOVING + STATIC + TRANSPARENT);
    for i in 0..MOVING {
        let center = Vec3::new(
            (i % 5) as f32 * 2.0 - 4.0 + drift,
            (i / 5) as f32 * 2.0 - 4.0,
            0.0,
        );
        objects.push(object(slots.sphere, slots.moving, center, Vec3::ONE));
    }
    for i in 0..STATIC {
        let center = Vec3::new(
            (i % 60) as f32 * 0.5 - 15.0,
            (i / 60) as f32 * 0.5 - 12.0,
            -6.0,
        );
        objects.push(object(slots.cube, slots.solid, center, Vec3::splat(0.4)));
    }
    for i in 0..TRANSPARENT {
        let center = Vec3::new((i % 10) as f32 - 4.5, (i / 10) as f32 - 2.0, -2.0);
        objects.push(object(
            slots.cube,
            slots.glass,
            center,
            Vec3::new(0.8, 0.8, 0.1),
        ));
    }
    objects
}

/// Median of the samples (the list is not empty).
fn median(mut values: Vec<f32>) -> f32 {
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

struct Sample {
    gpu_ms: f32,
    cpu_ms: f32,
    forward_draws: u32,
    object_draws: u32,
    gpu_timed: bool,
}

/// Renders the scene with TAA on, optionally without the object-velocity pass.
fn measure(adapter: &wgpu::Adapter, skip_object_velocity: bool) -> Option<Sample> {
    let mut renderer = common::renderer(adapter, SIZE)?;
    renderer.enable_taa(true);
    renderer.debug_skip_object_velocity(skip_object_velocity);
    let slots = Slots {
        sphere: renderer.upload_mesh(&create_sphere(0.5, SPHERE_SEGMENTS)),
        cube: renderer.upload_mesh(&create_cube()),
        solid: renderer.add_material(Material::solid(Vec3::new(0.7, 0.6, 0.5))),
        moving: renderer.add_material(Material::solid(Vec3::new(0.9, 0.3, 0.2))),
        glass: renderer.add_material(Material::solid(Vec3::new(0.2, 0.5, 0.9))),
    };
    renderer.material_blend_flags[slots.glass] = BlendMode::Alpha;
    let target = common::target(&renderer.device, SIZE);
    let camera = Camera::new(0.9, 1.0, 0.1, 500.0);
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 40.0), Quat::IDENTITY);
    renderer.update_camera_buffer();

    let mut gpu = Vec::with_capacity(FRAMES);
    let mut cpu = Vec::with_capacity(FRAMES);
    let mut draws = (0, 0);
    for step in 0..WARMUP + FRAMES {
        let objects = scene(&slots, step);
        let view = FrameView::new(&camera, &objects, &lights);
        renderer.render_frame_view(&view, &target).unwrap();
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        if step >= WARMUP {
            gpu.push(renderer.gpu_profiler.total_ms());
            cpu.push(renderer.last_frame_cpu_timings().total_ms());
        }
        draws = (
            renderer.forward_draw_calls(),
            renderer.debug_object_velocity_draws(),
        );
    }
    Some(Sample {
        gpu_ms: median(gpu),
        cpu_ms: median(cpu),
        forward_draws: draws.0,
        object_draws: draws.1,
        gpu_timed: renderer.gpu_profiler.is_enabled(),
    })
}

fn print_row(name: &str, sample: &Sample) {
    println!(
        "| {name} | {:.3} | {:.3} | {} | {} | {} |",
        sample.gpu_ms,
        sample.cpu_ms,
        sample.forward_draws,
        sample.object_draws,
        sample.forward_draws + sample.object_draws
    );
}

#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn object_velocity_pass_cost_table() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let (Some(with), Some(without)) = (measure(&adapter, false), measure(&adapter, true)) else {
        eprintln!("no device, skipping");
        return;
    };
    let draws_with = (with.forward_draws + with.object_draws) as f32;
    let draws_without = (without.forward_draws + without.object_draws) as f32;
    let draws_saved_percent = if draws_with > 0.0 {
        (draws_with - draws_without) / draws_with * 100.0
    } else {
        0.0
    };
    let gpu_saved_ms = with.gpu_ms - without.gpu_ms;

    println!(
        "| mode | GPU total ms (median) | CPU total ms (median) | forward draws | object-velocity draws | all draws |"
    );
    println!("|---|---|---|---|---|---|");
    print_row("(а) как сейчас", &with);
    print_row("(б) без прохода скорости объектов", &without);
    println!(
        "GPU timestamps available: {} (GPU column is 0 when not)",
        with.gpu_timed
    );
    println!("saving: GPU {gpu_saved_ms:.3} ms, draw calls {draws_saved_percent:.1} %");
    let merge = gpu_saved_ms >= GATE_GPU_MS || draws_saved_percent >= GATE_DRAWS_PERCENT;
    println!(
        "verdict (gate: GPU >= {GATE_GPU_MS} ms or draws >= {GATE_DRAWS_PERCENT} %): {}",
        if merge {
            "сливать"
        } else {
            "не сливать"
        }
    );
}

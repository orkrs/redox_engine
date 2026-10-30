//! VG-04 step 5: CPU cost and draw-call count of HLOD for a scene of 4000 small
//! objects in 40 cells, seen from about 300 m. Ignored by default; run with
//! `cargo test -p redox_core --test hlod_measure -- --ignored --nocapture`.
//!
//! The `hlod_frame` cost is measured on the CPU. When a GPU adapter exists the
//! same scene is also rendered headless and the renderer's `FrameCpuTimings`
//! (prepare, record, submit) are printed. Without an adapter that part is skipped.
//! The proxies are real: each cell's 100 actors are merged by `build_proxy` and uploaded
//! with `upload_proxy` (one mean colour per cell, VG-04 variant b). ASSUMPTION: the actors
//! of this bench have no meshes of their own, so each actor is a unit cube.

#![allow(clippy::unwrap_used)]

use std::time::Instant;

use redox_core::hlod_system::{HlodRuntime, hlod_frame};
use redox_ecs::{Entity, World};
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::hlod::{HlodConfig, HlodInput, HlodParams, HlodProxy, build_proxy};
use redox_render::hlod_gpu::{ProxyGpu, proxy_render_object, upload_proxy};
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::renderer::FrameCpuTimings;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;
use redox_scene::{ActorKindTable, PartitionCell, StreamedActor};

const KINDS: &str = r#"(kinds: { "prop": (mesh: "meshes/prop.glb", color: (0.5, 0.5, 0.5)) })"#;
const CELLS_X: i32 = 8;
const CELLS_Y: i32 = 5;
const PER_CELL: usize = 100;
const CELL_SIZE: f32 = 50.0;
const WARMUP_FRAMES: usize = 30;
const TIMED_FRAMES: usize = 100;
const GPU_SIZE: u32 = 128;
const GPU_FRAMES: usize = 5;

fn detail_object(id: u32, mesh: usize, material: usize) -> RenderObject {
    RenderObject {
        id,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::IDENTITY,
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::empty(),
    }
}

/// Spawns 40 cells of 100 actors and returns the entities.
fn spawn_scene(world: &mut World) -> Vec<Entity> {
    let mut entities = Vec::new();
    for x in 0..CELLS_X {
        for y in 0..CELLS_Y {
            let cell = PartitionCell { x, y };
            for _ in 0..PER_CELL {
                let id = format!("prop_{}", entities.len());
                let entity = world.spawn_bundle((StreamedActor {
                    cell,
                    id,
                    kind: "prop".into(),
                },));
                entities.push(entity);
            }
        }
    }
    entities
}

/// The proxy of one cell: the unit cubes of its actors merged by `build_proxy`, as `bake-hlod` does.
fn build_cell_proxy(cell: PartitionCell) -> HlodProxy {
    let cube = create_cube();
    let positions: Vec<[f32; 3]> = cube.vertices.iter().map(|v| v.position).collect();
    let origin = Vec3::new(cell.x as f32 * CELL_SIZE, 0.0, cell.y as f32 * CELL_SIZE);
    let transforms: Vec<Mat4> = (0..PER_CELL)
        .map(|i| {
            let offset = Vec3::new((i % 10) as f32 * 4.0 + 1.0, 0.5, (i / 10) as f32 * 4.0 + 1.0);
            Mat4::from_translation(origin + offset)
        })
        .collect();
    let inputs: Vec<HlodInput<'_>> = transforms
        .iter()
        .map(|&transform| HlodInput {
            positions: &positions,
            indices: &cube.indices,
            transform,
            color: [0.5, 0.5, 0.5],
            vertex_colors: None,
        })
        .collect();
    let params = HlodParams {
        target_triangles: 2000,
        max_error: 0.05 * CELL_SIZE,
        weld_epsilon: 1e-4,
    };
    build_proxy(&inputs, &params).expect("cell proxy")
}

/// Average microseconds of `hlod_frame` over `frames` frames, and the object count of the last one.
fn time_frames(
    world: &mut World,
    entities: &[Entity],
    viewer: Vec3,
    dt: f32,
    frames: usize,
) -> (f64, usize) {
    let mut total = 0.0;
    let mut count = 0;
    for _ in 0..frames {
        let mut objects: Vec<RenderObject> = entities
            .iter()
            .map(|e| detail_object(e.id(), 0, 0))
            .collect();
        let start = Instant::now();
        hlod_frame(world, viewer, dt, &mut objects);
        total += start.elapsed().as_secs_f64() * 1e6;
        count = objects.len();
    }
    (total / frames as f64, count)
}

/// Headless renderer with one cube mesh and one material, or None without an adapter.
struct GpuRig {
    renderer: Renderer,
    target: wgpu::TextureView,
    camera: Camera,
    lights: LightScene,
    cube: usize,
    material: usize,
}

fn gpu_rig() -> Option<GpuRig> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    let limits = if adapter.get_info().backend == wgpu::Backend::Vulkan {
        redox_render::renderer::required_device_limits(&adapter.limits())
    } else {
        wgpu::Limits::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .ok()?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width: GPU_SIZE,
        height: GPU_SIZE,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    let mut renderer = Renderer::from_shared_device(device, queue, &adapter, None, config);
    let cube = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.5, 0.3)));
    let camera = Camera::new(0.9, 1.0, 0.1, 500.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 40.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let target = renderer
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("hlod_measure_target"),
            size: wgpu::Extent3d {
                width: GPU_SIZE,
                height: GPU_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    Some(GpuRig {
        renderer,
        target,
        camera,
        lights,
        cube,
        material,
    })
}

/// Renders `GPU_FRAMES` frames of `objects` and returns the timings of the median frame.
fn gpu_median(rig: &mut GpuRig, objects: &[RenderObject]) -> FrameCpuTimings {
    let mut all = Vec::new();
    for _ in 0..GPU_FRAMES {
        let view = FrameView::new(&rig.camera, objects, &rig.lights);
        rig.renderer.render_frame_view(&view, &rig.target).unwrap();
        all.push(rig.renderer.last_frame_cpu_timings());
        let _ = rig
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }
    all.sort_by(|a, b| a.total_ms().total_cmp(&b.total_ms()));
    all[all.len() / 2]
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn four_thousand_objects_in_forty_cells_seen_from_300m() {
    let mut gpu = gpu_rig();
    let (cube, material) = gpu.as_ref().map_or((0, 0), |rig| (rig.cube, rig.material));

    let mut world = World::new();
    world.insert_resource(ActorKindTable::from_ron(KINDS).unwrap());
    let entities = spawn_scene(&mut world);
    // Viewer at x = -275: the nearest cell centre is about 300 m away.
    let viewer = Vec3::new(-275.0, 0.0, 125.0);

    let (without_us, without_draws) = time_frames(&mut world, &entities, viewer, 0.0, TIMED_FRAMES);

    let cells: Vec<PartitionCell> = (0..CELLS_X)
        .flat_map(|x| (0..CELLS_Y).map(move |y| PartitionCell { x, y }))
        .collect();
    let mut runtime = HlodRuntime::new(
        HlodConfig::default(),
        CELL_SIZE,
        Vec3::ZERO,
        cells.iter().copied(),
    );
    // Real proxies: built per cell, and uploaded when a GPU adapter exists. Without one the
    // runtime gets placeholder slots, which only the CPU part uses.
    let proxies: Vec<HlodProxy> = cells.iter().map(|cell| build_cell_proxy(*cell)).collect();
    let gpu_proxies: Option<Vec<ProxyGpu>> = gpu.as_mut().map(|rig| {
        proxies
            .iter()
            .map(|proxy| upload_proxy(&mut rig.renderer, proxy))
            .collect()
    });
    let placeholder = ProxyGpu {
        mesh_slot: cube,
        material_slot: material,
        bounds: Aabb::empty(),
    };
    for (i, cell) in cells.iter().enumerate() {
        let slot = gpu_proxies.as_ref().map_or(placeholder, |slots| slots[i]);
        runtime.register_proxy(*cell, slot);
    }
    world.insert_resource(runtime);
    let _ = time_frames(&mut world, &entities, viewer, 0.1, WARMUP_FRAMES);
    let (with_us, with_draws) = time_frames(&mut world, &entities, viewer, 0.0, TIMED_FRAMES);

    println!("HLOD measurement, 4000 objects in 40 cells at about 300 m");
    println!("  without HLOD: {without_draws} draw calls, hlod_frame {without_us:.1} us/frame");
    println!("  with HLOD:    {with_draws} draw calls, hlod_frame {with_us:.1} us/frame");
    assert_eq!(
        without_draws,
        CELLS_X as usize * CELLS_Y as usize * PER_CELL
    );
    assert_eq!(with_draws, CELLS_X as usize * CELLS_Y as usize);

    let Some(rig) = gpu.as_mut() else {
        eprintln!("no GPU adapter: renderer timings skipped");
        return;
    };
    let without_objects: Vec<RenderObject> = entities
        .iter()
        .map(|e| detail_object(e.id(), cube, material))
        .collect();
    let mut with_objects: Vec<RenderObject> = entities
        .iter()
        .map(|e| detail_object(e.id(), cube, material))
        .collect();
    let Some(slots) = gpu_proxies.as_deref() else {
        return;
    };
    hlod_frame(&mut world, viewer, 0.0, &mut with_objects);
    // The cells are in the proxy state here, so the proxies are drawn at full opacity.
    with_objects.extend(
        slots
            .iter()
            .enumerate()
            .map(|(i, slot)| proxy_render_object(slot, 100_000 + i as u32, 1.0)),
    );
    let without = gpu_median(rig, &without_objects);
    let with = gpu_median(rig, &with_objects);
    println!(
        "  renderer CPU per frame, without HLOD: {:.3} ms (prepare {:.3}, record {:.3}, submit {:.3})",
        without.total_ms(),
        without.prepare_ms,
        without.record_ms,
        without.submit_ms
    );
    println!(
        "  renderer CPU per frame, with HLOD:    {:.3} ms (prepare {:.3}, record {:.3}, submit {:.3})",
        with.total_ms(),
        with.prepare_ms,
        with.record_ms,
        with.submit_ms
    );
}

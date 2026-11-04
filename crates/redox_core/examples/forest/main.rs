//! Forest: a fir forest with grass, ferns and boulders (Poly Haven assets exported to `.glb`
//! with Blender) lit by a low sun through haze. It shows the engine's lighting stack on a
//! natural scene: virtual shadow maps with soft contact shadows, GTAO, IBL from an HDRI,
//! volumetric sun scatter, bloom and TAA.
//!
//! Needs `assets/forest/models/*.glb` and `assets/forest/phalzer_forest_01_4k.hdr` (not in git).
//!
//! Fly with WASD (Shift: faster, Space/C: up/down), look with the arrow keys.
//! Picture without a window: `... -- --shot forest.png [width height]`.

#![allow(clippy::unwrap_used)]

mod model;
mod scatter;

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use redox_core::config::EngineConfig;
use redox_core::window::{RunClosure, create_window};
use redox_math::{Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::mesh::{Mesh, Vertex};
use redox_render::post::fog::FogSettings;
use redox_render::post::post_fx::MotionBlurSettings;
use redox_render::post::ssr::SsrSettings;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::shadow::smrt::SmrtQuality;
use redox_render::{RenderObject, Renderer};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

use scatter::{Instance, World};

/// Size of the ground mesh, metres.
const GROUND_SIZE: f32 = 220.0;
/// Quads per side of the ground mesh.
const GROUND_CELLS: usize = 220;
/// Ground texture repeat, metres per tile.
const GROUND_TILE: f32 = 3.0;
/// Per-frame object limit of the renderer's model buffer (10 000), with a little to spare.
const MAX_OBJECTS: usize = 9_900;
/// Cosine of the half-angle of the cone kept around the view direction.
const VIEW_CONE_COS: f32 = 0.35;
const SHOT_SIZE: (u32, u32) = (1600, 900);
const FIELD_OF_VIEW: f32 = 0.95;

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/forest")
}

/// Everything loaded onto the GPU plus the placed instances.
struct Forest {
    models: Vec<model::Model>,
    ground: (usize, usize),
    world: World,
}

fn ground_mesh() -> Mesh {
    let n = GROUND_CELLS + 1;
    let step = GROUND_SIZE / GROUND_CELLS as f32;
    let at = |i: usize, j: usize| {
        let x = -GROUND_SIZE * 0.5 + i as f32 * step;
        let z = -GROUND_SIZE * 0.5 + j as f32 * step;
        [x, scatter::ground_height(x, z), z]
    };
    let positions: Vec<[f32; 3]> = (0..n * n).map(|k| at(k % n, k / n)).collect();
    let normals: Vec<[f32; 3]> = positions
        .iter()
        .map(|p| {
            let e = 0.5;
            let dx =
                scatter::ground_height(p[0] + e, p[2]) - scatter::ground_height(p[0] - e, p[2]);
            let dz =
                scatter::ground_height(p[0], p[2] + e) - scatter::ground_height(p[0], p[2] - e);
            Vec3::new(-dx, 2.0 * e, -dz).normalize().to_array()
        })
        .collect();
    let uvs: Vec<[f32; 2]> = positions
        .iter()
        .map(|p| [p[0] / GROUND_TILE, p[2] / GROUND_TILE])
        .collect();
    let mut indices = Vec::with_capacity(GROUND_CELLS * GROUND_CELLS * 6);
    for j in 0..GROUND_CELLS {
        for i in 0..GROUND_CELLS {
            let a = (j * n + i) as u32;
            let (b, c, d) = (a + 1, a + n as u32, a + n as u32 + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let tangents = model::tangents(&positions, &normals, &uvs, &indices);
    let vertices = (0..positions.len())
        .map(|i| Vertex {
            position: positions[i],
            normal: normals[i],
            uv: uvs[i],
            tangent: tangents[i],
        })
        .collect();
    Mesh::new(vertices, indices)
}

fn load_forest(r: &mut Renderer) -> Result<Forest, String> {
    let dir = assets_dir().join("models");
    let mut cache = model::TextureCache::default();
    let mut models = Vec::new();
    let mut index = std::collections::HashMap::new();
    for name in scatter::MODEL_NAMES {
        let tint = if name.starts_with("boulder") {
            Vec3::new(0.42, 0.47, 0.38)
        } else if name.starts_with("grass") || name.starts_with("fern") {
            Vec3::new(0.78, 1.0, 0.62)
        } else {
            Vec3::ONE
        };
        let loaded = model::load(r, &mut cache, &dir.join(format!("{name}.glb")), tint)?;
        index.insert(*name, models.len());
        models.push(loaded);
    }
    let ground_material = model::load(
        r,
        &mut cache,
        &dir.join("ground_a.glb"),
        Vec3::new(0.55, 0.72, 0.42),
    )?
    .parts[0]
        .material;
    let ground = (r.upload_mesh(&ground_mesh()), ground_material);
    let world = scatter::populate(&index, 7);
    Ok(Forest {
        models,
        ground,
        world,
    })
}

fn load_sky(r: &mut Renderer) {
    let name = std::env::var("FOREST_SKY").unwrap_or_else(|_| "skybox.hdr".into());
    let path = if name.starts_with("skybox") {
        assets_dir().join("..").join(&name)
    } else {
        assets_dir().join(&name)
    };
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("forest: no sky at {}", path.display());
        return;
    };
    match redox_render::resource::texture::Texture::from_hdr_bytes(
        &r.device, &r.queue, &bytes, "sky",
    ) {
        Ok(texture) => {
            r.set_environment(&texture);
            r.set_skybox(true);
        }
        Err(error) => eprintln!("forest: sky: {error}"),
    }
}

/// Renderer settings of the picture.
fn configure(r: &mut Renderer) {
    r.set_render_scale(1.0);
    r.set_ssao_enabled(true);
    r.bloom_enabled = true;
    r.taa_enabled = true;
    r.set_vsm_enabled(true);
    r.set_smrt_quality(SmrtQuality::Quality);
    r.set_exposure(0.55);
    r.set_ibl_intensity(0.9);
    r.set_ssr(Some(SsrSettings {
        intensity: 0.5,
        ..Default::default()
    }));
    r.set_fog(Some(FogSettings {
        color: [0.9, 0.88, 0.78],
        density: 0.0045,
        height_falloff: 0.1,
        sun_scatter: 1.0,
        anisotropy: 0.65,
        start_distance: 2.0,
        ..Default::default()
    }));
    r.set_fog_froxel(true);
    r.set_motion_blur(Some(MotionBlurSettings {
        shutter: 0.3,
        max_pixels: 10.0,
        taps: 8,
        spread_beyond_edges: false,
    }));
    load_sky(r);
}

fn lights() -> LightScene {
    // `direction` points towards the sun: low, in front of and left of the camera.
    let mut sun = DirectionalLight::new(
        Vec3::new(-0.55, 0.42, -0.72),
        Vec3::new(1.0, 0.85, 0.6),
        6.0,
    );
    sun.soft_shadows = true;
    sun.vsm = true;
    LightScene {
        directional: sun,
        ambient: Vec3::new(0.1, 0.14, 0.1),
        points: vec![],
        spots: vec![],
    }
}

/// Camera state of the fly view.
#[derive(Clone, Copy)]
struct Pose {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
}

impl Pose {
    fn start() -> Self {
        // `FOREST_CAM="x y z yaw pitch"` overrides the start pose (for close-up checks).
        if let Ok(text) = std::env::var("FOREST_CAM") {
            let v: Vec<f32> = text
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            if let [x, y, z, yaw, pitch] = v[..] {
                return Self {
                    pos: Vec3::new(x, y, z),
                    yaw,
                    pitch,
                };
            }
        }
        Self {
            pos: Vec3::new(0.0, scatter::ground_height(0.0, 0.0) + 1.35, 4.0),
            yaw: 0.35,
            pitch: 0.04,
        }
    }

    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }
}

/// Objects to draw for the camera at `pos`: models switch to their cheaper form with distance.
fn objects(r: &Renderer, forest: &Forest, pose: Pose) -> Vec<RenderObject> {
    let pos = pose.pos;
    let forward = pose.rotation() * Vec3::NEG_Z;
    let mut out = Vec::new();
    let mut push = |mesh: usize, material: usize, model: Mat4| {
        let Some(bounds) = r.get_mesh_bounds(mesh) else {
            return;
        };
        out.push(RenderObject {
            id: out.len() as u32,
            shadow_class: ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: model,
            color: [1.0; 4],
            mesh_index: mesh,
            material_index: material,
            bounds: bounds.transform(model),
        });
    };
    push(forest.ground.0, forest.ground.1, Mat4::IDENTITY);
    for Instance {
        near,
        far,
        switch,
        cull,
        pos: at,
        yaw,
        scale,
    } in &forest.world.instances
    {
        let to = *at - pos;
        let distance = to.length();
        // Beyond a few metres, what is behind the camera or far off to the side is skipped
        // here: the renderer takes at most `MAX_OBJECTS` objects a frame.
        if distance > *cull || (distance > 9.0 && to.dot(forward) < distance * VIEW_CONE_COS) {
            continue;
        }
        let model_index = if distance < *switch { *near } else { *far };
        let matrix = Mat4::from_scale_rotation_translation(
            Vec3::splat(*scale),
            Quat::from_rotation_y(*yaw),
            *at,
        );
        for part in &forest.models[model_index].parts {
            push(part.mesh, part.material, matrix);
        }
    }
    out.truncate(MAX_OBJECTS);
    out
}

fn draw(
    r: &mut Renderer,
    forest: &Forest,
    pose: Pose,
    aspect: f32,
    view: &wgpu::TextureView,
) -> Result<(), String> {
    let camera = Camera::new(FIELD_OF_VIEW, aspect, 0.1, 400.0);
    r.camera_uniform.update(&camera, pose.pos, pose.rotation());
    r.update_camera_buffer();
    let objects = objects(r, forest, pose);
    let lights = lights();
    let frame = FrameView::new(&camera, &objects, &lights);
    r.render_frame_view(&frame, view)
        .map_err(|error| format!("frame: {error:?}"))
}

fn screenshot(path: &str, size: (u32, u32)) -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|error| format!("no GPU adapter: {error:?}"))?;
    let wanted = wgpu::Features::PRIMITIVE_INDEX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | wgpu::Features::TEXTURE_COMPRESSION_BC
        | wgpu::Features::PIPELINE_CACHE
        | wgpu::Features::INDIRECT_FIRST_INSTANCE
        | redox_render::profiler::PROFILER_FEATURES;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wanted,
        required_limits: redox_render::renderer::required_device_limits(&adapter.limits()),
        ..Default::default()
    }))
    .map_err(|error| format!("no device: {error:?}"))?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Rgba8Unorm,
        width: size.0,
        height: size.1,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: Default::default(),
    };
    let mut r = Renderer::from_shared_device(device, queue, &adapter, None, config);
    let started = Instant::now();
    configure(&mut r);
    let forest = load_forest(&mut r)?;
    for tree in forest.world.instances.iter().take(6) {
        println!(
            "tree at {:.1} {:.1} {:.1}",
            tree.pos.x, tree.pos.y, tree.pos.z
        );
    }
    println!(
        "loaded {} instances in {:.1} s",
        forest.world.instances.len(),
        started.elapsed().as_secs_f32()
    );
    let target = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("forest_shot"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
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
    let aspect = size.0 as f32 / size.1 as f32;
    // Frames let TAA, shadow pages and fog history settle before the read-back.
    for _ in 0..16 {
        draw(&mut r, &forest, Pose::start(), aspect, &view)?;
        let _ = r.device.poll(wgpu::PollType::wait_indefinitely());
    }
    let pixels = read_texture_ktx2(&r.device, &r.queue, &target, false)
        .ok_or("read back: no pixels")?
        .levels
        .swap_remove(0);
    image::save_buffer(path, &pixels, size.0, size.1, image::ColorType::Rgba8)
        .map_err(|error| format!("save {path}: {error}"))?;
    println!("saved {path}");
    Ok(())
}

fn play() -> Result<(), String> {
    let config = EngineConfig {
        window_title: "RedOx - Forest (WASD, arrows)".to_string(),
        ..Default::default()
    };
    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(
        window.clone(),
        config.wgpu_present_mode(),
    ));
    configure(&mut ctx);
    let forest = load_forest(&mut ctx)?;
    let mut pose = Pose::start();
    let mut held: HashSet<KeyCode> = HashSet::new();
    let mut last = Instant::now();

    event_loop
        .run_closure(move |event, elwt| match event {
            winit::event::Event::WindowEvent { event, .. } => match event {
                WindowEvent::KeyboardInput { event: key, .. } => {
                    if let PhysicalKey::Code(code) = key.physical_key {
                        if key.state == ElementState::Pressed {
                            held.insert(code);
                        } else {
                            held.remove(&code);
                        }
                    }
                }
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::Resized(size) => ctx.resize(size.width, size.height),
                WindowEvent::RedrawRequested => {
                    let now = Instant::now();
                    let dt = (now - last).as_secs_f32().min(0.1);
                    last = now;
                    pose = fly(pose, &held, dt);
                    let output = match redox_render::acquire_surface_texture(ctx.surface()) {
                        Ok(output) => output,
                        Err(
                            redox_render::SurfaceError::Outdated | redox_render::SurfaceError::Lost,
                        ) => {
                            let (w, h) = (ctx.config.width, ctx.config.height);
                            ctx.resize(w, h);
                            return;
                        }
                        Err(_) => return,
                    };
                    let view = output.texture.create_view(&Default::default());
                    let aspect = ctx.config.width as f32 / ctx.config.height.max(1) as f32;
                    if let Err(error) = draw(&mut ctx, &forest, pose, aspect, &view) {
                        eprintln!("forest: {error}");
                    }
                    ctx.queue().present(output);
                }
                _ => {}
            },
            winit::event::Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .map_err(|error| format!("event loop: {error:?}"))
}

/// Moves the camera by the held keys.
fn fly(pose: Pose, held: &HashSet<KeyCode>, dt: f32) -> Pose {
    let key = |c: KeyCode| f32::from(u8::from(held.contains(&c)));
    let speed = if held.contains(&KeyCode::ShiftLeft) {
        9.0
    } else {
        3.0
    };
    let turn = 1.4 * dt;
    let yaw = pose.yaw + turn * (key(KeyCode::ArrowLeft) - key(KeyCode::ArrowRight));
    let pitch =
        (pose.pitch + turn * (key(KeyCode::ArrowUp) - key(KeyCode::ArrowDown))).clamp(-1.4, 1.4);
    let forward = Quat::from_rotation_y(yaw) * Vec3::NEG_Z;
    let right = Quat::from_rotation_y(yaw) * Vec3::X;
    let delta = forward * (key(KeyCode::KeyW) - key(KeyCode::KeyS))
        + right * (key(KeyCode::KeyD) - key(KeyCode::KeyA))
        + Vec3::Y * (key(KeyCode::Space) - key(KeyCode::KeyC));
    Pose {
        pos: pose.pos + delta * speed * dt,
        yaw,
        pitch,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().position(|a| a == "--shot") {
        Some(at) => {
            let path = args.get(at + 1).map_or("forest.png", String::as_str);
            let dim = |k: usize| args.get(at + k).and_then(|v| v.parse::<u32>().ok());
            let size = match (dim(2), dim(3)) {
                (Some(w), Some(h)) => (w, h),
                _ => SHOT_SIZE,
            };
            screenshot(path, size)
        }
        None => play(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("forest: {error}");
            ExitCode::FAILURE
        }
    }
}

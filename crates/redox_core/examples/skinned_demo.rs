//! Skinned animation demo: clip-driven rigs plus a two-bone IK arm that reaches for a moving
//! target. Pose sampling, blending and skinning run on the CPU (`redox_anim`); the deformed
//! vertices are streamed into ordinary mesh slots so shadows, GTAO and TAA all see them.
//!
//! Run:         cargo run --example skinned_demo -p redox_core --release
//! Screenshots: cargo run --example skinned_demo -p redox_core --release -- --screenshot <dir>
//!              (renders offscreen, no window, writes PNG files)

use std::path::PathBuf;
use std::time::Instant;

use redox_anim::procedural::{chain_model, demo_rig};
use redox_anim::{apply_two_bone_ik, Animator, Pose, SkinnedModel};
use redox_core::config::EngineConfig;
use redox_core::window::create_window;
use redox_input::InputState;
use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::context::RenderContext;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene, PointLight};
use redox_render::material::Material;
use redox_render::mesh::primitive;
use redox_render::{RenderObject, Renderer, SkinnedMeshBinding};
use winit::keyboard::KeyCode;

const SHOT_SIZE: (u32, u32) = (1280, 720);
/// Animation times captured in `--screenshot` mode.
const SHOT_TIMES: [f32; 4] = [0.0, 0.6, 1.2, 1.8];
/// Frames rendered per screenshot so TAA history converges.
const SHOT_WARMUP_FRAMES: usize = 10;
const CAMERA_TARGET: Vec3 = Vec3::new(0.0, 1.1, 0.0);

struct Rig {
    model: SkinnedModel,
    binding: SkinnedMeshBinding,
    animator: Animator,
    material: usize,
    base: Vec3,
    /// `true`: the first three bones are solved by IK toward the moving target.
    ik: bool,
}

struct DemoScene {
    rigs: Vec<Rig>,
    ground_mesh: usize,
    ground_mat: usize,
    target_mesh: usize,
    target_mat: usize,
    target_world: Vec3,
    /// Static reference crate: shows the scene's shadows next to the deforming meshes.
    crate_mat: usize,
}

impl DemoScene {
    fn new(r: &mut Renderer) -> Self {
        let ground_mesh = r.upload_mesh(&primitive::create_cube());
        let ground_mat = r.add_material(Material::solid(Vec3::new(0.30, 0.31, 0.34)));
        let target_mesh = r.upload_mesh(&primitive::create_sphere(0.14, 20));
        let mut glow = Material::solid(Vec3::new(1.0, 0.55, 0.15));
        glow.emissive = Vec3::new(2.0, 0.9, 0.2);
        let target_mat = r.add_material(glow);

        let crate_mat = r.add_material(Material::solid(Vec3::new(0.55, 0.38, 0.22)));

        let rigs = vec![
            make_rig(r, demo_rig(), 0, Vec3::new(-3.0, 0.0, 0.0), Vec3::new(0.15, 0.65, 0.65), false),
            make_rig(r, demo_rig(), 1, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.80, 0.25, 0.60), false),
            make_rig(
                r,
                chain_model(2, 1.0, 0.14, 16),
                0,
                Vec3::new(3.0, 0.0, 0.0),
                Vec3::new(0.90, 0.72, 0.20),
                true,
            ),
        ];
        Self { rigs, ground_mesh, ground_mat, target_mesh, target_mat, target_world: Vec3::ZERO, crate_mat }
    }

    /// Poses every rig for animation time `t` (seconds) and uploads the deformed meshes.
    fn update(&mut self, r: &Renderer, t: f32) {
        for rig in &mut self.rigs {
            let skeleton = &rig.model.skeleton;
            let local = if rig.ik {
                let target_world = rig.base + Vec3::new(0.9 * (0.9 * t).cos(), 0.9 + 0.5 * (0.7 * t).sin(), 0.6 * (0.9 * t).sin());
                self.target_world = target_world;
                let mut local = skeleton.bind_local_pose();
                // Pole points at the camera so the elbow bends toward the viewer.
                apply_two_bone_ik(skeleton, &mut local, [0, 1, 2], target_world - rig.base, Vec3::new(0.0, 1.0, 4.0));
                local
            } else {
                rig.animator.time = t;
                rig.animator.evaluate(skeleton, &rig.model.clips)
            };
            let skin = Pose::from_local(skeleton, &local).skin_matrices(skeleton);
            if !rig.binding.update(r, &skin) {
                log::warn!("skinned mesh update rejected");
            }
            if rig.binding.last_overshoot() > 0.0 {
                log::warn!("skinned mesh left its quantisation bounds by {}", rig.binding.last_overshoot());
            }
        }
    }

    fn objects(&self, r: &Renderer) -> Vec<RenderObject> {
        let mut out = Vec::new();
        let ground = Mat4::from_scale_rotation_translation(Vec3::new(24.0, 0.25, 24.0), Quat::IDENTITY, Vec3::new(0.0, -0.125, 0.0));
        out.push(object(r, ground, self.ground_mesh, self.ground_mat));
        out.push(object(r, Mat4::from_translation(self.target_world), self.target_mesh, self.target_mat));
        let crate_model = Mat4::from_scale_rotation_translation(Vec3::splat(0.9), Quat::from_rotation_y(0.5), Vec3::new(-1.6, 0.45, 1.8));
        out.push(object(r, crate_model, self.ground_mesh, self.crate_mat));
        for rig in &self.rigs {
            let model = Mat4::from_translation(rig.base);
            out.push(RenderObject {
                model_matrix: model,
                color: [1.0, 1.0, 1.0, 1.0],
                mesh_index: rig.binding.mesh_slot,
                material_index: rig.material,
                bounds: rig.binding.bounds.transform(model),
            });
        }
        out
    }
}

fn make_rig(r: &mut Renderer, model: SkinnedModel, clip: usize, base: Vec3, color: Vec3, ik: bool) -> Rig {
    let bind = model
        .geometry
        .positions
        .iter()
        .fold(Aabb::empty(), |b, p| b.expand(Vec3::from(*p)));
    // Animation can swing vertices about as far as the rig is long.
    let margin = 2.0 * bind.half_extents().max_element();
    let binding = SkinnedMeshBinding::new(r, &model.geometry, margin);
    let mut material = Material::solid(color);
    material.roughness = 0.42;
    material.metallic = 0.1;
    let material = r.add_material(material);
    Rig { model, binding, animator: Animator { clip, ..Default::default() }, material, base, ik }
}

fn object(r: &Renderer, model: Mat4, mesh: usize, material: usize) -> RenderObject {
    let bounds = r.get_mesh_bounds(mesh).map_or(
        Aabb { min: Vec3::splat(-1.0e9), max: Vec3::splat(1.0e9) },
        |b| b.transform(model),
    );
    RenderObject { model_matrix: model, color: [1.0; 4], mesh_index: mesh, material_index: material, bounds }
}

/// Camera slowly swinging around the scene.
fn camera_pose(t: f32) -> (Vec3, Quat) {
    let yaw = 0.45 * (0.3 * t).sin();
    let pos = Vec3::new(8.5 * yaw.sin(), 2.8, 8.5 * yaw.cos());
    let forward = (CAMERA_TARGET - pos).normalize();
    (pos, Quat::from_rotation_arc(Vec3::NEG_Z, forward))
}

/// Point-light shadows are re-rendered every frame, so they follow deforming meshes exactly.
fn shadowed(mut light: PointLight) -> PointLight {
    light.cast_shadows = true;
    light
}

fn light_scene() -> LightScene {
    let mut sun = DirectionalLight::new(Vec3::new(-0.45, 1.0, 0.55), Vec3::new(1.0, 0.96, 0.90), 1.5);
    // The directional VSM caches shadow pages until an object moves more than 0.5 m, so it
    // would show stale shadows for in-place deformation. The sun therefore casts no shadow
    // here; the point lights below re-render their shadows every frame.
    sun.soft_shadows = false;
    sun.vsm = false;
    LightScene {
        directional: sun,
        ambient: Vec3::splat(0.08),
        points: vec![
            shadowed(PointLight::new(Vec3::new(-4.0, 2.6, 3.0), Vec3::new(0.35, 0.55, 1.0), 40.0, 12.0)),
            shadowed(PointLight::new(Vec3::new(4.0, 2.6, 3.0), Vec3::new(1.0, 0.55, 0.30), 40.0, 12.0)),
        ],
        spots: vec![],
    }
}

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    match args.iter().position(|a| a == "--screenshot") {
        Some(i) => {
            let dir = args.get(i + 1).map_or_else(|| PathBuf::from("skinned_demo_shots"), PathBuf::from);
            screenshots(&dir);
        }
        None => windowed(),
    }
}

fn windowed() {
    let config = EngineConfig { window_title: "RedOx — skinned animation demo".to_string(), ..Default::default() };
    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(window.clone(), config.wgpu_present_mode()));
    ctx.set_render_scale(config.render_scale);
    let mut scene = DemoScene::new(&mut ctx);
    let lights = light_scene();
    let mut input = InputState::new();
    let (start, mut last) = (Instant::now(), Instant::now());

    event_loop
        .run(move |event, elwt| match event {
            winit::event::Event::NewEvents(_) => input.begin_frame(),
            winit::event::Event::WindowEvent { event, .. } => {
                input.process_window_event(&event);
                match event {
                    winit::event::WindowEvent::CloseRequested => elwt.exit(),
                    winit::event::WindowEvent::Resized(size) => ctx.resize(size.width, size.height),
                    winit::event::WindowEvent::RedrawRequested => {
                        if input.keyboard.is_pressed(KeyCode::Escape) {
                            elwt.exit();
                        }
                        let _dt = last.elapsed().as_secs_f32().min(0.05);
                        last = Instant::now();
                        let t = start.elapsed().as_secs_f32();

                        scene.update(&ctx, t);
                        let (pos, rot) = camera_pose(t);
                        let camera = Camera::new(0.9, ctx.config.width as f32 / ctx.config.height as f32, 0.1, 100.0);
                        ctx.camera_uniform.update(&camera, pos, rot);
                        ctx.update_camera_buffer();

                        let objects = scene.objects(&ctx);
                        let fv = FrameView::new(&camera, &objects, &lights);
                        let output = ctx.surface().get_current_texture().unwrap();
                        let view = output.texture.create_view(&Default::default());
                        ctx.render_frame_view(&fv, &view).unwrap();
                        output.present();
                    }
                    _ => {}
                }
            }
            winit::event::Event::AboutToWait => window.request_redraw(),
            _ => {}
        })
        .unwrap();
}

/// Offscreen render: no window, one PNG per entry of [`SHOT_TIMES`].
fn screenshots(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).expect("create screenshot dir");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter");
    let wanted = wgpu::Features::SHADER_PRIMITIVE_INDEX | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("skinned_demo_offscreen"),
            required_features: adapter.features() & wanted,
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .expect("device");

    let (w, h) = SHOT_SIZE;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: w,
        height: h,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    let mut renderer = Renderer::from_shared_device(device, queue, &adapter, None, config);
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("skinned_demo_target"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());

    let mut scene = DemoScene::new(&mut renderer);
    let lights = light_scene();
    let camera = Camera::new(0.9, w as f32 / h as f32, 0.1, 100.0);
    for t in SHOT_TIMES {
        let (pos, rot) = camera_pose(0.0);
        for _ in 0..SHOT_WARMUP_FRAMES {
            scene.update(&renderer, t);
            renderer.camera_uniform.update(&camera, pos, rot);
            renderer.update_camera_buffer();
            let objects = scene.objects(&renderer);
            let fv = FrameView::new(&camera, &objects, &lights);
            renderer.render_frame_view(&fv, &view).expect("render");
        }
        let pixels = read_texture(&renderer.device, &renderer.queue, &target, w, h);
        let path = dir.join(format!("skinned_demo_t{:03}.png", (t * 100.0) as u32));
        image::save_buffer(&path, &pixels, w, h, image::ColorType::Rgba8).expect("write png");
        println!("wrote {}", path.display());
    }
}

fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
    let bytes_per_row = w * 4;
    assert_eq!(bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0, "width must keep rows 256-byte aligned");
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot_readback"),
        size: u64::from(bytes_per_row * h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("map screenshot buffer"));
    device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range().to_vec();
    buffer.unmap();
    data
}

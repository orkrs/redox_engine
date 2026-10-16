//! Skinned animation demo: clip-driven rigs plus a two-bone IK arm that reaches for a moving
//! target. Pose sampling, blending and skinning run on the CPU (`redox_anim`); the deformed
//! vertices are streamed into ordinary mesh slots so shadows, GTAO and TAA all see them.
//!
//! Keys (window title shows the state): V = sun shadows (VSM / off), P = point-light shadows (off by
//!       default: they duplicate the sun shadows),
//!       D = re-render sun shadows under deforming meshes every frame, Esc = quit.
//!
//! Run:         cargo run --example skinned_demo -p redox_core --release
//! Screenshots: cargo run --example skinned_demo -p redox_core --release -- --screenshot <dir>
//!              (renders offscreen, no window, writes PNG files; add `--size 1920x1080`,
//!              `--times 5,10.5`, `--sun vsm|off`)
//! Regression:  ... -- --golden crates/redox_core/tests/golden
//!              renders three fixed frames and compares them with the stored images (exit code
//!              1 on a mismatch); add `--update-golden` to rewrite the references.

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
/// `--golden` renders these frames at this size (small: the references live in the repo).
const GOLDEN_SIZE: (u32, u32) = (640, 360);
const GOLDEN_TIMES: [f32; 3] = [2.0, 6.0, 11.0];
/// A golden frame passes when its mean absolute channel error stays below this (0..255)...
const GOLDEN_MAX_MEAN_ERROR: f32 = 1.5;
/// ...and fewer than this fraction of pixels differ by more than [`GOLDEN_PIXEL_TOLERANCE`].
const GOLDEN_MAX_BAD_FRACTION: f32 = 0.005;
const GOLDEN_PIXEL_TOLERANCE: u8 = 24;
/// Animation times captured in `--screenshot` mode.
const SHOT_TIMES: [f32; 5] = [1.0, 5.0, 10.0, 15.0, 20.0];
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
        let ground_mat = r.add_material(Material::solid(Vec3::new(0.15, 0.17, 0.20)));
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

    /// World-space bounds of every deformed rig (for shadow invalidation).
    fn deformed_world_bounds(&self) -> Vec<Aabb> {
        self.rigs
            .iter()
            .map(|rig| rig.binding.deformed_bounds().transform(Mat4::from_translation(rig.base)))
            .collect()
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
/// Quality settings shared by the window and the screenshot mode.
fn configure_renderer(r: &mut Renderer) {
    r.set_ssao_enabled(true);
    r.bloom_enabled = true;
    r.set_exposure(0.9);
    r.set_ibl_intensity(0.55);
    load_environment(r);
}

/// HDR sky + image-based lighting from `assets/skybox.hdr` (skipped when the file is missing).
fn load_environment(r: &mut Renderer) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/skybox.hdr");
    let Ok(bytes) = std::fs::read(&path) else {
        log::warn!("no environment at {}; using the flat clear colour", path.display());
        return;
    };
    match redox_render::resource::texture::Texture::from_hdr_bytes(&r.device, &r.queue, &bytes, "skybox") {
        Ok(texture) => {
            r.set_environment(&texture);
            r.set_skybox(true);
        }
        Err(e) => log::warn!("failed to decode {}: {e}", path.display()),
    }
}

fn camera_pose(t: f32) -> (Vec3, Quat) {
    let yaw = 0.45 * (0.3 * t).sin();
    let pos = Vec3::new(8.5 * yaw.sin(), 2.8, 8.5 * yaw.cos());
    let forward = (CAMERA_TARGET - pos).normalize();
    (pos, Quat::from_rotation_arc(Vec3::NEG_Z, forward))
}

/// Point-light shadows are re-rendered every frame, so they follow deforming meshes exactly.
fn shadowed(mut light: PointLight, enabled: bool) -> PointLight {
    light.cast_shadows = enabled;
    light
}

/// How the sun's shadow is produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SunShadow {
    Off,
    /// Virtual shadow map clipmap (cached pages).
    Vsm,
}

impl SunShadow {
    fn next(self) -> Self {
        match self {
            Self::Vsm => Self::Off,
            Self::Off => Self::Vsm,
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "vsm" => Some(Self::Vsm),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

fn light_scene(sun_shadow: SunShadow, point_shadows: bool) -> LightScene {
    let mut sun = DirectionalLight::new(Vec3::new(-0.45, 1.0, 0.55), Vec3::new(1.0, 0.94, 0.84), 2.2);
    // `vsm` routes the sun through the VSM clipmap; without it the sun casts no shadow.
    sun.soft_shadows = false;
    sun.vsm = sun_shadow == SunShadow::Vsm;
    LightScene {
        directional: sun,
        ambient: Vec3::new(0.10, 0.12, 0.16),
        points: vec![
            shadowed(PointLight::new(Vec3::new(-4.0, 2.6, 3.0), Vec3::new(0.35, 0.55, 1.0), 26.0, 12.0), point_shadows),
            shadowed(PointLight::new(Vec3::new(4.0, 2.6, 3.0), Vec3::new(1.0, 0.55, 0.30), 26.0, 12.0), point_shadows),
        ],
        spots: vec![],
    }
}

fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--golden") {
        let dir = args.get(i + 1).map_or_else(|| PathBuf::from("golden"), PathBuf::from);
        let update = args.iter().any(|a| a == "--update-golden");
        std::process::exit(if golden(&dir, update, &args) { 0 } else { 1 });
    }
    match args.iter().position(|a| a == "--screenshot") {
        Some(i) => {
            let dir = args.get(i + 1).map_or_else(|| PathBuf::from("skinned_demo_shots"), PathBuf::from);
            let size = args
                .iter()
                .position(|a| a == "--size")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.split_once('x'))
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .unwrap_or(SHOT_SIZE);
            screenshots(&dir, size, &args);
        }
        None => windowed(),
    }
}

fn windowed() {
    let config = EngineConfig { window_title: "RedOx — skinned animation demo".to_string(), ..Default::default() };
    let (event_loop, window) = create_window(&config);
    let mut ctx = pollster::block_on(RenderContext::new(window.clone(), config.wgpu_present_mode()));
    ctx.set_render_scale(config.render_scale);
    configure_renderer(&mut ctx);
    let mut scene = DemoScene::new(&mut ctx);
    let mut input = InputState::new();
    let (mut sun_shadows, mut point_shadows, mut follow_deformation) = (SunShadow::Vsm, false, true);
    let title = |sun: SunShadow, point: bool, follow: bool| {
        format!("RedOx skinned demo | V sun shadows: {sun:?} | P point shadows: {} | D follow deformation: {}", on_off(point), on_off(follow))
    };
    window.set_title(&title(sun_shadows, point_shadows, follow_deformation));
    let (start, mut last) = (Instant::now(), Instant::now());
    let mut frame_count: u64 = 0;

    event_loop
        .run(move |event, elwt| match event {
            winit::event::Event::NewEvents(_) => input.begin_frame(),
            winit::event::Event::WindowEvent { event, .. } => {
                input.process_window_event(&event);
                if let winit::event::WindowEvent::KeyboardInput { event: key, .. } = &event {
                    // Handled here, not via `just_pressed`: that flag is cleared on the
                    // next `NewEvents`, which comes before the redraw that would read it.
                    if key.state == winit::event::ElementState::Pressed && !key.repeat {
                        if let winit::keyboard::PhysicalKey::Code(code) = key.physical_key {
                            let changed = match code {
                                KeyCode::KeyV => {
                                    sun_shadows = sun_shadows.next();
                                    true
                                }
                                KeyCode::KeyP => {
                                    point_shadows = !point_shadows;
                                    true
                                }
                                KeyCode::KeyD => {
                                    follow_deformation = !follow_deformation;
                                    true
                                }
                                _ => false,
                            };
                            if changed {
                                ctx.set_vsm_enabled(sun_shadows == SunShadow::Vsm);
                                window.set_title(&title(sun_shadows, point_shadows, follow_deformation));
                            }
                        }
                    }
                }
                match event {
                    winit::event::WindowEvent::CloseRequested => elwt.exit(),
                    winit::event::WindowEvent::Resized(size) => ctx.resize(size.width, size.height),
                    winit::event::WindowEvent::RedrawRequested => {
                        if input.keyboard.is_pressed(KeyCode::Escape) {
                            elwt.exit();
                        }
                        frame_count += 1;
                        if frame_count % 120 == 0 {
                            eprintln!("gpu: {}", ctx.gpu_profiler.report());
                            let (stats, free, capacity) = ctx.vsm_diagnostics();
                            eprintln!(
                                "vsm: pool free {free}/{capacity}, allocated {}, overflow {}, dirty last frame {}",
                                stats.pages_allocated, stats.pages_overflow, stats.pages_dirty
                            );
                        }
                        let _dt = last.elapsed().as_secs_f32().min(0.05);
                        last = Instant::now();
                        let t = start.elapsed().as_secs_f32();

                        scene.update(&ctx, t);
                        if follow_deformation {
                            ctx.invalidate_shadow_region(&scene.deformed_world_bounds());
                        }
                        let lights = light_scene(sun_shadows, point_shadows);
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
fn screenshots(dir: &std::path::Path, size: (u32, u32), args: &[String]) -> Vec<(usize, PathBuf)> {
    let mut written = Vec::new();
    std::fs::create_dir_all(dir).expect("create screenshot dir");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter");
    let wanted = wgpu::Features::SHADER_PRIMITIVE_INDEX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
        | redox_render::PROFILER_FEATURES;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("skinned_demo_offscreen"),
            required_features: adapter.features() & wanted,
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .expect("device");

    let (w, h) = size;
    // Same sRGB swapchain-style format as the window, so tone mapping and output encoding match.
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
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

    configure_renderer(&mut renderer);
    let mut scene = DemoScene::new(&mut renderer);
    let sun = args
        .iter()
        .position(|a| a == "--sun")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| SunShadow::parse(v))
        .unwrap_or(SunShadow::Vsm);
    renderer.set_vsm_enabled(sun == SunShadow::Vsm);
    let lights = light_scene(sun, false);
    let camera = Camera::new(0.9, w as f32 / h as f32, 0.1, 100.0);
    // One continuous 60 fps timeline from t = 0, exactly like the windowed loop, so shadow
    // page streaming, TAA history and camera motion behave the same as on screen.
    // `--times 10,10.02,10.04` overrides the default capture times (seconds).
    let times: Vec<f32> = args
        .iter()
        .position(|a| a == "--times")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
        .filter(|v: &Vec<f32>| !v.is_empty())
        .unwrap_or_else(|| SHOT_TIMES.to_vec());
    let shot_frames: Vec<usize> = times.iter().map(|t| (t * 60.0).round() as usize).collect();
    let last_frame = shot_frames.iter().copied().max().unwrap_or(0);
    for frame in 0..=last_frame {
        let t = frame as f32 / 60.0;
        let (pos, rot) = camera_pose(t);
        scene.update(&renderer, t);
        renderer.invalidate_shadow_region(&scene.deformed_world_bounds());
        renderer.camera_uniform.update(&camera, pos, rot);
        renderer.update_camera_buffer();
        let objects = scene.objects(&renderer);
        let fv = FrameView::new(&camera, &objects, &lights);
        renderer.render_frame_view(&fv, &view).expect("render");
        // Frame pacing like a presented swapchain: without it the CPU runs arbitrarily far
        // ahead of the GPU and per-frame staging memory can be reused while still in flight.
        renderer.device.poll(wgpu::Maintain::Wait);

        if shot_frames.contains(&frame) {
            println!("gpu: {}", renderer.gpu_profiler.report());
            let (stats, free, capacity) = renderer.vsm_diagnostics();
            println!(
                "t={t:.1}s vsm: pool free {free}/{capacity}, allocated {}, freed {}, overflow {}, dirty {}",
                stats.pages_allocated, stats.pages_freed, stats.pages_overflow, stats.pages_dirty
            );
            let pixels = read_texture(&renderer.device, &renderer.queue, &target, w, h);
            let path = dir.join(format!("skinned_demo_f{frame:05}.png"));
            image::save_buffer(&path, &pixels, w, h, image::ColorType::Rgba8).expect("write png");
            println!("wrote {}", path.display());
            written.push((frame, path));
        }
    }
    written
}

/// Renders the golden frames and compares (or, with `update`, stores) them. `true` = all match.
fn golden(dir: &std::path::Path, update: bool, args: &[String]) -> bool {
    let work = std::env::temp_dir().join(format!("redox_golden_{}", std::process::id()));
    let mut golden_args: Vec<String> = args
        .iter()
        .filter(|a| !matches!(a.as_str(), "--golden" | "--update-golden"))
        .cloned()
        .collect();
    let times = GOLDEN_TIMES.map(|t| t.to_string()).join(",");
    golden_args.extend(["--times".to_string(), times]);
    let frames = screenshots(&work, GOLDEN_SIZE, &golden_args);
    std::fs::create_dir_all(dir).expect("create golden dir");

    let mut ok = true;
    for (frame, path) in frames {
        let reference = dir.join(format!("frame_{frame:05}.png"));
        if update || !reference.exists() {
            std::fs::copy(&path, &reference).expect("store golden frame");
            println!("golden: stored {}", reference.display());
            continue;
        }
        let (Ok(a), Ok(b)) = (image::open(&path), image::open(&reference)) else {
            println!("golden: frame {frame}: cannot read images");
            ok = false;
            continue;
        };
        let (a, b) = (a.to_rgba8(), b.to_rgba8());
        if a.dimensions() != b.dimensions() {
            println!("golden: frame {frame}: size differs");
            ok = false;
            continue;
        }
        let diff = image_difference(a.as_raw(), b.as_raw());
        let pass = diff.mean_error <= GOLDEN_MAX_MEAN_ERROR && diff.bad_fraction <= GOLDEN_MAX_BAD_FRACTION;
        println!(
            "golden: frame {frame}: mean error {:.2}, {:.3}% pixels off -> {}",
            diff.mean_error,
            diff.bad_fraction * 100.0,
            if pass { "ok" } else { "MISMATCH" }
        );
        ok &= pass;
    }
    let _ = std::fs::remove_dir_all(&work);
    ok
}

/// How far two RGBA8 images are apart.
#[derive(Debug, PartialEq)]
struct ImageDifference {
    /// Mean absolute error over the RGB channels, 0..255.
    mean_error: f32,
    /// Fraction of pixels where some channel differs by more than [`GOLDEN_PIXEL_TOLERANCE`].
    bad_fraction: f32,
}

fn image_difference(a: &[u8], b: &[u8]) -> ImageDifference {
    assert_eq!(a.len(), b.len(), "images must have the same size");
    let pixels = (a.len() / 4).max(1);
    let (mut sum, mut bad) = (0u64, 0usize);
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let mut worst = 0u8;
        for c in 0..3 {
            let d = pa[c].abs_diff(pb[c]);
            sum += u64::from(d);
            worst = worst.max(d);
        }
        bad += usize::from(worst > GOLDEN_PIXEL_TOLERANCE);
    }
    ImageDifference {
        mean_error: sum as f32 / (pixels * 3) as f32,
        bad_fraction: bad as f32 / pixels as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_no_difference() {
        let img = vec![10u8, 20, 30, 255, 200, 100, 0, 255];
        assert_eq!(image_difference(&img, &img), ImageDifference { mean_error: 0.0, bad_fraction: 0.0 });
    }

    #[test]
    fn small_noise_stays_under_the_thresholds() {
        let a = vec![100u8; 4 * 1000];
        let b: Vec<u8> = a.iter().map(|v| v + 1).collect();
        let d = image_difference(&a, &b);
        assert!(d.mean_error <= GOLDEN_MAX_MEAN_ERROR && d.bad_fraction <= GOLDEN_MAX_BAD_FRACTION);
    }

    #[test]
    fn a_missing_shadow_block_is_caught() {
        let a = vec![200u8; 4 * 10_000];
        let mut b = a.clone();
        // 5 % of the pixels turn dark, like a wrongly shadowed page.
        for px in b.chunks_exact_mut(4).take(500) {
            px[..3].fill(80);
        }
        let d = image_difference(&a, &b);
        assert!(d.bad_fraction > GOLDEN_MAX_BAD_FRACTION, "{d:?}");
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

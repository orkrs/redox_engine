//! A DAG asset drawn by the real frame (VG-02): traversal, visibility buffer and resolve.

mod common;

use redox_math::{Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_sphere;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::virtual_geometry::{DagParams, VGConfig, build_vg_dag_asset};

const SIZE: u32 = 128;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn target(renderer: &Renderer) -> Target {
    let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vg_dag_frame_target"),
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
    let view = texture.create_view(&Default::default());
    Target { texture, view }
}

/// How the sphere of a test scene is registered.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Path {
    /// DAG asset with `VGConfig::dag_traverse` on.
    DagOn,
    /// DAG asset with the default config (flag off): registered but not drawn.
    DagOffDefault,
}

/// Renders 8 frames of the scene and returns the last one. With `sphere`, a sphere (radius 1) is
/// placed at that position on the given path; the camera is on +Z looking towards -Z. `None` is
/// skipped without a GPU.
fn frame_with(sphere: Option<Vec3>, path: Path) -> Option<Vec<u8>> {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return None;
    };
    // The resolve pass reads the material table: it needs at least the material of the asset.
    let material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.3, 0.2)));
    let config = match path {
        Path::DagOn => VGConfig {
            dag_traverse: true,
            ..VGConfig::default()
        },
        Path::DagOffDefault => VGConfig::default(),
    };
    renderer.init_virtual_geometry(config);
    if let Some(at) = sphere {
        let mesh = create_sphere(1.0, 48);
        let transform = Mat4::from_translation(at).to_cols_array_2d();
        let vg = renderer.vg_system.as_mut().expect("vg");
        match path {
            Path::DagOn | Path::DagOffDefault => {
                let asset = build_vg_dag_asset(&mesh, &DagParams::default(), material as u32)
                    .expect("dag asset");
                let id = vg
                    .register_dag_asset(asset, &renderer.queue)
                    .expect("register");
                vg.spawn_dag_instance(id, transform).expect("spawn");
            }
        }
    }

    let target = target(&renderer);
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    let mut last = Vec::new();
    for _ in 0..8 {
        renderer
            .camera_uniform
            .update(&camera, Vec3::new(0.0, 0.0, 6.0), Quat::IDENTITY);
        renderer.update_camera_buffer();
        let frame = FrameView::new(&camera, &[], &lights);
        renderer
            .render_frame_view(&frame, &target.view)
            .expect("frame");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        last = read_texture_ktx2(&renderer.device, &renderer.queue, &target.texture, false)
            .expect("readback")
            .levels
            .swap_remove(0);
    }
    if let Some(vg) = renderer.vg_system.as_ref() {
        eprintln!(
            "vg: dag instances {} draws {} has_content {}",
            vg.dag_instance_count(),
            vg.dag_draw_count(),
            vg.has_content()
        );
    }
    Some(last)
}

/// Shorthand: the DAG path with the flag on.
fn frame(sphere: Option<Vec3>) -> Option<Vec<u8>> {
    frame_with(sphere, Path::DagOn)
}

/// Pixels whose colour differs clearly between two frames.
fn differing_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(x, y)| x.iter().zip(*y).any(|(p, q)| p.abs_diff(*q) > 12))
        .count()
}

#[test]
fn a_dag_sphere_in_front_of_the_camera_is_drawn() {
    let (Some(empty), Some(with)) = (frame(None), frame(Some(Vec3::ZERO))) else {
        return;
    };
    let covered = differing_pixels(&empty, &with);
    eprintln!("dag sphere: {covered} pixels differ from the empty scene");
    // Radius 1 at distance 6 with a 0.9 rad field of view: about 8 % of the frame.
    assert!(
        covered > 600,
        "the sphere must cover many pixels, got {covered}"
    );
    assert!(
        covered < 4000,
        "the sphere is not the whole frame: {covered}"
    );
}

#[test]
fn a_dag_sphere_outside_the_view_is_not_drawn() {
    let (Some(empty), Some(with)) = (frame(None), frame(Some(Vec3::new(60.0, 0.0, 0.0)))) else {
        return;
    };
    let covered = differing_pixels(&empty, &with);
    assert!(
        covered < 20,
        "an off-screen sphere changed {covered} pixels"
    );
}

/// With the flag off (the default) a DAG sphere in front of the camera is not drawn.
#[test]
fn the_dag_path_is_off_by_default() {
    let (Some(empty), Some(with)) = (
        frame_with(None, Path::DagOffDefault),
        frame_with(Some(Vec3::ZERO), Path::DagOffDefault),
    ) else {
        return;
    };
    let covered = differing_pixels(&empty, &with);
    assert!(
        covered < 20,
        "with the flag off the DAG sphere changed {covered} pixels"
    );
}

/// The DAG frame (flag on) covers about as many pixels as the analytic disc of the sphere.
/// A sphere of radius 1 at distance 6 seen with a 0.9 rad field of view projects to a disc of
/// `pi * r_px^2` pixels; the tessellated, LOD-reduced silhouette must stay within 10 % of it.
/// (The flat meshlet path draws nothing in this scene setup, so it is no usable reference.)
#[test]
fn the_dag_frame_matches_the_analytic_disc_within_tolerance() {
    let (Some(empty), Some(dag)) = (
        frame_with(None, Path::DagOn),
        frame_with(Some(Vec3::ZERO), Path::DagOn),
    ) else {
        return;
    };
    let covered = differing_pixels(&empty, &dag);
    let px_per_metre = SIZE as f32 * 0.5 / (6.0 * (0.9f32 * 0.5).tan());
    let expected = (std::f32::consts::PI * px_per_metre * px_per_metre) as usize;
    eprintln!("dag {covered} px, analytic disc {expected} px");
    assert!(
        covered.abs_diff(expected) * 10 <= expected,
        "DAG {covered} px vs analytic {expected} px differ by more than 10 %"
    );
}

/// VG-02 benchmark: 1000 instances of a sphere of about 100k triangles. Prints the GPU time of
/// the passes (when the adapter has timestamps) and the wall time of a frame. Target: the
/// traversal pass (`cluster_vg_cull`) at most 1 ms; the coordinator records the number in
/// `docs/BENCHMARKS.md`.
#[test]
#[ignore = "timing: run with --release -- --ignored --nocapture"]
fn dag_bench_thousand_spheres() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        return;
    };
    let size = 512;
    let Some(mut renderer) = common::renderer(&adapter, size) else {
        return;
    };
    let material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.3, 0.2)));
    renderer.init_virtual_geometry(VGConfig {
        max_vertices: 1 << 20,
        max_indices: 1 << 22,
        max_meshlets: 1 << 18,
        dag_traverse: true,
        ..VGConfig::default()
    });
    let mesh = create_sphere(1.0, 316);
    eprintln!("sphere: {} triangles", mesh.indices.len() / 3);
    let built = std::time::Instant::now();
    let asset =
        build_vg_dag_asset(&mesh, &DagParams::default(), material as u32).expect("dag asset");
    eprintln!("dag build: {:.1} s", built.elapsed().as_secs_f32());
    let vg = renderer.vg_system.as_mut().expect("vg");
    let id = vg
        .register_dag_asset(asset, &renderer.queue)
        .expect("register");
    for i in 0..1000u32 {
        let at = Vec3::new(
            (i % 10) as f32 * 3.0 - 13.5,
            ((i / 10) % 10) as f32 * 3.0 - 13.5,
            -((i / 100) as f32) * 6.0,
        );
        let transform = Mat4::from_translation(at).to_cols_array_2d();
        vg.spawn_dag_instance(id, transform).expect("spawn");
    }
    let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vg_dag_bench_target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let camera = Camera::new(0.9, 1.0, 0.1, 400.0);
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    let frames = 30;
    let mut wall = std::time::Duration::ZERO;
    for n in 0..(frames + 5) {
        renderer
            .camera_uniform
            .update(&camera, Vec3::new(0.0, 0.0, 30.0), Quat::IDENTITY);
        renderer.update_camera_buffer();
        let frame = FrameView::new(&camera, &[], &lights);
        let started = std::time::Instant::now();
        renderer.render_frame_view(&frame, &view).expect("frame");
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        if n >= 5 {
            wall += started.elapsed();
        }
    }
    eprintln!(
        "1000 spheres: {:.2} ms wall per frame, dag draws {}",
        wall.as_secs_f64() * 1000.0 / f64::from(frames),
        renderer
            .vg_system
            .as_ref()
            .map_or(0, |v| v.dag_draw_count())
    );
    eprintln!("{}", renderer.gpu_profiler.report());
    if let Some(vg) = renderer.vg_system.as_ref() {
        eprintln!("vg gpu bytes (capacity): {}", vg.gpu_bytes());
    }
}

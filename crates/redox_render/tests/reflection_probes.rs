//! Reflection probe slots, captures and shader lookups on a real device (skipped without a GPU).

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::reflection::{MAX_PROBES, ReflectionProbe, ReflectionProbeSet};
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
/// Bytes per texel of the probe format (Rgba16Float).
const TEXEL_BYTES: u32 = 8;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn probe(center: Vec3, priority: i32) -> ReflectionProbe {
    ReflectionProbe {
        center,
        half_extent: Vec3::splat(10.0),
        blend_distance: 1.0,
        priority,
    }
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f32::from(bits & 0x3ff);
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 if fraction == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + fraction / 1024.0) * 2f32.powi(exponent - 15),
    }
}

/// Reads texel `(x, y)` of mip 0 of one cube-array layer, as linear RGBA.
fn read_probe_texel(
    renderer: &Renderer,
    set: &ReflectionProbeSet,
    layer: u32,
    x: u32,
    y: u32,
) -> [f32; 4] {
    let device = &renderer.device;
    let row_bytes = SIZE * TEXEL_BYTES;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe_readback"),
        size: u64::from(row_bytes) * u64::from(SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: set.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().expect("map callback").expect("map read");
    let data = slice.get_mapped_range().expect("mapped range");
    let offset = (y * row_bytes + x * TEXEL_BYTES) as usize;
    let texel: Vec<f32> = (0..4)
        .map(|c| {
            let at = offset + c * 2;
            f16_to_f32(u16::from_le_bytes([data[at], data[at + 1]]))
        })
        .collect();
    drop(data);
    buffer.unmap();
    [texel[0], texel[1], texel[2], texel[3]]
}

/// Scene shared by the render tests: a metal floor at y = -1 and a red wall at x = 4.
struct Scene {
    objects: Vec<RenderObject>,
    lights: LightScene,
    camera: Camera,
    target: wgpu::Texture,
    view: wgpu::TextureView,
}

fn object(mesh: usize, material: usize, id: u32, center: Vec3, scale: Vec3) -> RenderObject {
    RenderObject {
        id,
        shadow_class: ShadowClass::Auto,
        model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(center, scale),
    }
}

fn scene(renderer: &mut Renderer) -> Scene {
    let cube = renderer.upload_mesh(&create_cube());
    let floor_material = renderer.add_material(
        Material::solid(Vec3::splat(0.6))
            .metallic(1.0)
            .roughness(0.1),
    );
    let wall_material = renderer.add_material(Material::solid(Vec3::new(1.0, 0.0, 0.0)));
    let objects = vec![
        object(
            cube,
            floor_material,
            1,
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(20.0, 0.1, 20.0),
        ),
        object(
            cube,
            wall_material,
            2,
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::splat(2.0),
        ),
    ];
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.4);
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("reflection_test_target"),
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
    Scene {
        objects,
        lights,
        camera: Camera::new(0.9, 1.0, 0.1, 200.0),
        target,
        view,
    }
}

/// Renders the scene from `eye` looking along `forward`; returns the RGBA8 bytes.
/// Renders 16 frames and returns the last. The dither pattern of the shader repeats every 16
/// frames, and shadow pages settle after the first frame, so two images taken 16 frames apart are
/// comparable bit for bit; images of neighbouring frames differ by a level or two.
fn render(renderer: &mut Renderer, scene: &Scene, eye: Vec3, forward: Vec3) -> Vec<u8> {
    for _ in 0..15 {
        let _ = render_once(renderer, scene, eye, forward);
    }
    render_once(renderer, scene, eye, forward)
}

fn render_once(renderer: &mut Renderer, scene: &Scene, eye: Vec3, forward: Vec3) -> Vec<u8> {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, forward.normalize());
    renderer.camera_uniform.update(&scene.camera, eye, rotation);
    renderer.update_camera_buffer();
    let frame = FrameView::new(&scene.camera, &scene.objects, &scene.lights);
    renderer
        .render_frame_view(&frame, &scene.view)
        .expect("frame");
    let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
    read_texture_ktx2(&renderer.device, &renderer.queue, &scene.target, false)
        .expect("readback")
        .levels
        .swap_remove(0)
}

fn red_sum(frame: &[u8]) -> u64 {
    frame.chunks_exact(4).map(|px| u64::from(px[0])).sum()
}

fn max_channel_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// Eye and look direction that see the floor reflecting the +X wall.
fn reflection_view() -> (Vec3, Vec3) {
    let eye = Vec3::new(-2.0, 1.0, 0.0);
    (eye, Vec3::new(3.0, -1.9, 0.0))
}

#[test]
fn set_probes_keeps_at_most_four_by_priority() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some((device, _queue)) = common::device_with_bc(&adapter) else {
        eprintln!("no device, skipping");
        return;
    };
    let mut set = ReflectionProbeSet::new(&device);
    let probes: Vec<_> = (0..6).map(|p| probe(Vec3::ZERO, p)).collect();

    let accepted = set.set_probes(&probes);

    assert_eq!(accepted, MAX_PROBES);
    let kept: Vec<i32> = set.probes().iter().map(|p| p.priority).collect();
    assert_eq!(kept, vec![5, 4, 3, 2]);
}

#[test]
fn the_probe_array_is_counted_in_the_memory_report() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return;
    };
    let textures = renderer
        .memory_usage()
        .used(redox_render::MemoryCategory::Textures);
    assert!(textures >= renderer.reflection_probes.gpu_bytes());
}

#[test]
fn a_captured_probe_contains_the_colour_of_the_wall_it_faces() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return;
    };
    let scene = scene(&mut renderer);
    renderer.set_reflection_probes(&[probe(Vec3::ZERO, 0)]);

    renderer
        .capture_reflection_probe(0, &scene.objects, &scene.lights)
        .expect("capture");

    let texel = read_probe_texel(
        &renderer,
        &renderer.reflection_probes,
        0,
        SIZE / 2,
        SIZE / 2,
    );
    assert!(texel[0] > 0.5, "red channel {}", texel[0]);
    assert!(texel[2] < 0.2, "blue channel {}", texel[2]);
}

#[test]
fn capture_restores_the_renderer_settings() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return;
    };
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    let before_camera = bytemuck::bytes_of(&renderer.camera_uniform).to_vec();
    let before_light = bytemuck::bytes_of(&renderer.pbr_pass_light_uniform()).to_vec();
    renderer.set_reflection_probes(&[probe(Vec3::ZERO, 0)]);
    let lights = LightScene::default();

    renderer
        .capture_reflection_probe(0, &[], &lights)
        .expect("capture");

    assert_eq!(
        bytemuck::bytes_of(&renderer.camera_uniform),
        &before_camera[..]
    );
    assert_eq!(
        bytemuck::bytes_of(&renderer.pbr_pass_light_uniform()),
        &before_light[..]
    );
}

#[test]
fn the_frame_is_unchanged_without_probes() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return;
    };
    let scene = scene(&mut renderer);
    let (eye, forward) = reflection_view();
    let without = render(&mut renderer, &scene, eye, forward);

    renderer.set_reflection_probes_enabled(true);
    renderer.set_reflection_probes(&[]);
    let enabled_but_empty = render(&mut renderer, &scene, eye, forward);

    let differing = without
        .iter()
        .zip(&enabled_but_empty)
        .filter(|(a, b)| a != b)
        .count();
    // Two images 16 frames apart still differ by one level in a handful of bytes (temporal
    // filtering); the probe path must not add more than that.
    assert!(
        differing * 200 <= without.len() && max_channel_diff(&without, &enabled_but_empty) <= 1,
        "{differing} of {} bytes differ, largest difference {}",
        without.len(),
        max_channel_diff(&without, &enabled_but_empty)
    );
}

#[test]
fn a_glossy_floor_reflects_the_probe_wall_instead_of_the_sky() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return;
    };
    let scene = scene(&mut renderer);
    let (eye, forward) = reflection_view();
    let without = render(&mut renderer, &scene, eye, forward);

    renderer.set_reflection_probes(&[probe(Vec3::ZERO, 0)]);
    renderer
        .capture_reflection_probe(0, &scene.objects, &scene.lights)
        .expect("capture");
    renderer.set_reflection_probes_enabled(true);
    let with = render(&mut renderer, &scene, eye, forward);

    assert!(
        red_sum(&with) > red_sum(&without),
        "probe {} vs global {}",
        red_sum(&with),
        red_sum(&without)
    );
}

#[test]
fn parallax_moves_the_reflection_with_the_viewer() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return;
    };
    let scene = scene(&mut renderer);
    renderer.set_reflection_probes(&[probe(Vec3::ZERO, 0)]);
    renderer
        .capture_reflection_probe(0, &scene.objects, &scene.lights)
        .expect("capture");
    renderer.set_reflection_probes_enabled(true);

    let (eye, forward) = reflection_view();
    let first = render(&mut renderer, &scene, eye, forward);
    let moved_eye = eye + Vec3::new(0.0, 0.0, 1.0);
    let second = render(&mut renderer, &scene, moved_eye, forward);

    // Weak check: the reflected wall edge moves with the viewer, so the frames differ.
    assert_ne!(first, second);
}

#[test]
fn outside_the_box_the_global_ibl_is_used() {
    let _guard = lock();
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        eprintln!("no device, skipping");
        return;
    };
    let scene = scene(&mut renderer);
    let (eye, forward) = reflection_view();

    // The box is smaller than the floor distance: every floor pixel lies outside it. The capture
    // happens first so that its side effects (shadow pages re-centred on the probe) are in both images.
    let tiny = ReflectionProbe {
        center: Vec3::ZERO,
        half_extent: Vec3::splat(0.5),
        blend_distance: 0.1,
        priority: 0,
    };
    renderer.set_reflection_probes(&[tiny]);
    renderer
        .capture_reflection_probe(0, &scene.objects, &scene.lights)
        .expect("capture");
    let without = render(&mut renderer, &scene, eye, forward);
    renderer.set_reflection_probes_enabled(true);
    let with = render(&mut renderer, &scene, eye, forward);

    let differing = without.iter().zip(&with).filter(|(a, b)| a != b).count();
    assert!(
        max_channel_diff(&without, &with) <= 1,
        "{differing} bytes differ, largest difference {}",
        max_channel_diff(&without, &with)
    );
}

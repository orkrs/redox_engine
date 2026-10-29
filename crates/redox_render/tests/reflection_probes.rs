//! Reflection probe slots and captures on a real device (skipped without a GPU).

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::reflection::{MAX_PROBES, ReflectionProbe, ReflectionProbeSet};
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
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    set: &ReflectionProbeSet,
    layer: u32,
    x: u32,
    y: u32,
) -> [f32; 4] {
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
            origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
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
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().expect("map callback").expect("map read");
    let data = slice.get_mapped_range();
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
    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(1.0, 0.0, 0.0)));
    let wall = RenderObject {
        id: 1,
        shadow_class: ShadowClass::Auto,
        model_matrix: Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::IDENTITY,
            Vec3::new(4.0, 0.0, 0.0),
        ),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(Vec3::new(4.0, 0.0, 0.0), Vec3::splat(2.0)),
    };
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.4);
    renderer
        .reflection_probes
        .set_probes(&[probe(Vec3::ZERO, 0)]);

    renderer
        .capture_reflection_probe(0, &[wall], &lights)
        .expect("capture");

    let texel = read_probe_texel(
        &renderer.device,
        &renderer.queue,
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
    renderer
        .reflection_probes
        .set_probes(&[probe(Vec3::ZERO, 0)]);
    let lights = LightScene::default();

    renderer
        .capture_reflection_probe(0, &[], &lights)
        .expect("capture");

    assert_eq!(bytemuck::bytes_of(&renderer.camera_uniform), &before_camera[..]);
    assert_eq!(
        bytemuck::bytes_of(&renderer.pbr_pass_light_uniform()),
        &before_light[..]
    );
}

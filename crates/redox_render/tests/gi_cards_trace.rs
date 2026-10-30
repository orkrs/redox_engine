//! GPU tests for reading surface cards in the probe trace (GI-02 step 4). Each test returns early
//! when no adapter is available.

mod common;

use redox_math::{Aabb, Mat4, Vec3};
use redox_render::gi::card_atlas::CardAtlas;
use redox_render::gi::card_capture::{CardMaterial, ObjectCards, capture_object};
use redox_render::gi::card_light::{CardLightInputs, CardLighter};
use redox_render::gi::grid_gpu::GiGridGpu;
use redox_render::gi::trace::{GiTraceParams, GiTracer, TraceInputs};
use redox_render::mesh::{Mesh, Vertex};
use redox_render::sdf::global::{GlobalSdf, SdfBox};
use redox_render::sdf::global_gpu::GlobalSdfGpu;

/// Atlas edge for the tests.
const ATLAS: u32 = 256;
/// Voxels per axis of the test SDF.
const DIMS: [u32; 3] = [64, 64, 64];
/// Rays per probe in the tests.
const RAYS: u32 = 64;

fn setup() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = common::adapter()?;
    common::device_with_bc(&adapter)
}

/// Red cube from `-1` to `1` on every axis.
fn red_box() -> SdfBox {
    SdfBox {
        min: Vec3::splat(-1.0),
        max: Vec3::splat(1.0),
        albedo: Vec3::new(1.0, 0.0, 0.0),
        emissive: Vec3::ZERO,
    }
}

fn cube_mesh(half: f32) -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let mut n = [0.0f32; 3];
            n[axis] = sign;
            let corner = |su: f32, sv: f32| {
                let mut p = [0.0f32; 3];
                p[axis] = sign * half;
                p[u] = su * half;
                p[v] = sv * half;
                Vertex {
                    position: p,
                    normal: n,
                    uv: [0.0, 0.0],
                    tangent: [1.0, 0.0, 0.0],
                }
            };
            let base = vertices.len() as u32;
            vertices.extend([
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ]);
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    Mesh::new(vertices, indices)
}

fn red_cards(id: u32) -> (ObjectCards, u32) {
    let cards = capture_object(
        &cube_mesh(1.0),
        &Mat4::IDENTITY,
        &CardMaterial {
            albedo: Vec3::new(1.0, 0.0, 0.0),
            emissive: Vec3::ZERO,
        },
        0.1,
    );
    (cards, id)
}

/// Uniform environment cube (the trace samples it for escaping rays).
fn sky_cube(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test_sky"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let data: Vec<u8> = [51u8, 102, 230, 255]
        .iter()
        .copied()
        .cycle()
        .take(4 * 4 * 6 * 4)
        .collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
        },
        wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 6,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

/// Zeroed previous-frame grid, irradiance and probe offsets: the bounce term is zero.
struct NoPrevious {
    grid: wgpu::Buffer,
    irradiance: wgpu::TextureView,
    offsets: wgpu::Buffer,
}

fn no_previous(device: &wgpu::Device, queue: &wgpu::Queue) -> NoPrevious {
    let grid = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_prev_grid"),
        size: std::mem::size_of::<GiGridGpu>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&grid, 0, &vec![0u8; std::mem::size_of::<GiGridGpu>()]);
    let irradiance = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("test_irradiance"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let offsets = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_probe_offset"),
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    NoPrevious {
        grid,
        irradiance,
        offsets,
    }
}

/// The red cube scene: SDF on the GPU, the cards lit by the sun from `-X` (4 frames: every row).
fn lit_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    cards: &[(ObjectCards, u32)],
    sun_dir: Vec3,
) -> (GlobalSdfGpu, CardAtlas) {
    let bounds = Aabb {
        min: Vec3::splat(-3.0),
        max: Vec3::splat(3.0),
    };
    let sdf = GlobalSdf::from_boxes(&[red_box()], bounds, DIMS);
    let mut gpu_sdf = GlobalSdfGpu::new(device, queue, DIMS);
    gpu_sdf.upload(device, queue, &sdf);
    let mut atlas = CardAtlas::new(device, ATLAS);
    atlas.set_objects(device, queue, cards);
    let lighter = CardLighter::new(device);
    let previous = no_previous(device, queue);
    for frame in 0..4 {
        let mut encoder = device.create_command_encoder(&Default::default());
        lighter.record(
            device,
            queue,
            &mut encoder,
            &atlas,
            &CardLightInputs {
                sdf: &gpu_sdf,
                prev_grid: &previous.grid,
                prev_irradiance: &previous.irradiance,
                probe_offset: &previous.offsets,
                sun_dir,
                sun_radiance: Vec3::ONE,
                points: &[],
                frame_index: frame,
            },
        );
        queue.submit([encoder.finish()]);
    }
    (gpu_sdf, atlas)
}

/// Traces one probe at `(-2.5, 0, 0)` towards the red cube. Returns `[r, g, b, distance]` per ray.
fn trace_probe(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gpu_sdf: &GlobalSdfGpu,
    cards: Option<&CardAtlas>,
    sun_dir: Vec3,
) -> Vec<[f32; 4]> {
    let sky = sky_cube(device, queue);
    let previous = no_previous(device, queue);
    let tracer = GiTracer::new(device, 1);
    let params = GiTraceParams {
        grid_origin: Vec3::new(-2.5, 0.0, 0.0),
        spacing: 1.0,
        dims: [1, 1, 1],
        rays: RAYS,
        frame_index: 0,
        sun_dir,
        sun_radiance: Vec3::ONE,
        scroll: [0; 3],
        probe_base: 0,
    };
    let inputs = TraceInputs {
        sdf: gpu_sdf,
        sky: &sky,
        prev_grid: &previous.grid,
        prev_irradiance: &previous.irradiance,
        probe_offset: &previous.offsets,
        cards,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    tracer.record(device, queue, &mut encoder, &params, &inputs);
    let size = u64::from(RAYS) * 16;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_ray_readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&tracer.ray_data, 0, &staging, 0, size);
    queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().expect("map callback").expect("map");
    let data = slice.get_mapped_range().expect("mapped range");
    let rays = bytemuck::cast_slice::<u8, [f32; 4]>(&data).to_vec();
    drop(data);
    staging.unmap();
    rays
}

#[test]
fn a_ray_that_hits_a_red_box_returns_red_from_its_card() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let cards = [red_cards(0)];
    let sun = Vec3::new(-1.0, 0.0, 0.0);
    let (gpu_sdf, atlas) = lit_scene(&device, &queue, &cards, sun);
    let rays = trace_probe(&device, &queue, &gpu_sdf, Some(&atlas), sun);
    let hits: Vec<_> = rays.iter().filter(|r| r[3] < 40.0).collect();
    assert!(!hits.is_empty(), "no ray hit the box");
    assert!(hits.iter().any(|r| r[0] > 0.05), "no lit card hit");
    for r in hits {
        assert!(r[0] > 2.0 * r[1], "hit is not red: {r:?}");
    }
}

#[test]
fn a_missing_card_falls_back_to_the_probe_path() {
    let Some((device, queue)) = setup() else {
        return;
    };
    // The box has id 0, but the only card belongs to id 5: object 0 takes the probe path.
    let cards = [red_cards(5)];
    let sun = Vec3::new(-1.0, 0.0, 0.0);
    let (gpu_sdf, atlas) = lit_scene(&device, &queue, &cards, sun);
    let with_atlas = trace_probe(&device, &queue, &gpu_sdf, Some(&atlas), sun);
    let without = trace_probe(&device, &queue, &gpu_sdf, None, sun);
    for (a, b) in with_atlas.iter().zip(&without) {
        for c in 0..4 {
            let tol = 0.01 * b[c].abs().max(1e-3);
            assert!((a[c] - b[c]).abs() <= tol, "{a:?} vs {b:?}");
        }
    }
}

#[test]
fn disabling_cards_matches_the_previous_frame_bitwise() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let sun = Vec3::new(-1.0, 0.0, 0.0);
    let (gpu_sdf, _) = lit_scene(&device, &queue, &[], sun);
    // An atlas with no objects is what GI-01 looks like to the trace: no card is ever read.
    let empty = CardAtlas::new(&device, ATLAS);
    let off = trace_probe(&device, &queue, &gpu_sdf, None, sun);
    let empty_on = trace_probe(&device, &queue, &gpu_sdf, Some(&empty), sun);
    assert_eq!(off, empty_on);
}

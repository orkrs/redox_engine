//! Cost of surface cards (GI-02 step 5). Ignored by default; run with
//! `cargo test -p redox_render --test gi_cards_bench --release -- --ignored --nocapture`.
//!
//! Scene: 200 static cubes on a grid, cards at 0.25 m texels, the global SDF at 128³, and the
//! two probe cascades of Medium quality (64 rays). Times are wall clock around submit and a full
//! device poll, averaged over 90 frames; they include the CPU submit cost. The GPU timers of the
//! profiler (`profiler.rs`) give the per-pass numbers and are the ones to quote in BENCHMARKS.md.

mod common;

use std::time::Instant;

use redox_math::{Aabb, Mat4, Vec3};
use redox_render::gi::card_atlas::CardAtlas;
use redox_render::gi::card_capture::{CardMaterial, ObjectCards, capture_object};
use redox_render::gi::card_light::{CardLightInputs, CardLighter};
use redox_render::gi::grid_gpu::GiGridGpu;
use redox_render::gi::trace::{GiTraceParams, GiTracer, TraceInputs};
use redox_render::mesh::{Mesh, Vertex};
use redox_render::sdf::global::{GLOBAL_SDF_DIMS, GlobalSdf, SdfBox};
use redox_render::sdf::global_gpu::GlobalSdfGpu;

/// Static objects in the scene.
const OBJECTS: u32 = 200;
/// Frames averaged.
const FRAMES: u32 = 90;
/// Probes per cascade and rays per probe (Medium quality).
const CASCADE_DIMS: [u32; 3] = [16, 8, 16];
const RAYS: u32 = 64;
/// Card texel size in metres.
const TEXEL: f32 = 0.25;

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

/// Grid position of object `i` on the floor plane.
fn object_centre(i: u32) -> Vec3 {
    let x = (i % 20) as f32 * 1.5 - 15.0;
    let z = (i / 20) as f32 * 1.5 - 7.5;
    Vec3::new(x, 0.5, z)
}

#[test]
#[ignore = "benchmark: run with --ignored --nocapture in release"]
fn gi_cards_cost_for_200_static_objects() {
    let Some(adapter) = common::adapter() else {
        return;
    };
    let Some((device, queue)) = common::device_with_bc(&adapter) else {
        return;
    };
    let mesh = cube_mesh(0.5);
    let material = CardMaterial {
        albedo: Vec3::splat(0.6),
        emissive: Vec3::ZERO,
    };
    let mut boxes = Vec::new();
    let mut cards: Vec<(ObjectCards, u32)> = Vec::new();
    for i in 0..OBJECTS {
        let c = object_centre(i);
        boxes.push(SdfBox {
            min: c - Vec3::splat(0.5),
            max: c + Vec3::splat(0.5),
            albedo: material.albedo,
            emissive: Vec3::ZERO,
        });
        let model = Mat4::from_translation(c);
        cards.push((capture_object(&mesh, &model, &material, TEXEL), i));
    }
    let bounds = Aabb {
        min: Vec3::new(-17.0, -1.0, -9.0),
        max: Vec3::new(17.0, 4.0, 9.0),
    };
    let sdf = GlobalSdf::from_boxes(&boxes, bounds, GLOBAL_SDF_DIMS);
    let mut gpu_sdf = GlobalSdfGpu::new(&device, &queue, GLOBAL_SDF_DIMS);
    gpu_sdf.upload(&device, &queue, &sdf);

    let mut atlas = CardAtlas::new(&device, redox_render::gi::card_atlas::CARD_ATLAS_SIZE);
    atlas.set_objects(&device, &queue, &cards);
    println!(
        "card atlas: {} MiB, table {} entries, {} faces dropped",
        atlas.memory_bytes() / (1024 * 1024),
        atlas.table_len,
        atlas.dropped_faces
    );

    let probes_per_cascade: u32 = CASCADE_DIMS.iter().product();
    let tracer = GiTracer::new(&device, probes_per_cascade * 2);
    let lighter = CardLighter::new(&device);
    let sky = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("bench_sky"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
    let grid = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bench_grid"),
        size: std::mem::size_of::<GiGridGpu>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&grid, 0, &vec![0u8; std::mem::size_of::<GiGridGpu>()]);
    let irradiance = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("bench_irradiance"),
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
        label: Some("bench_offsets"),
        size: u64::from(probes_per_cascade * 2) * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let sun = Vec3::new(0.4, 0.8, 0.3).normalize();
    let mut total_with = 0.0f64;
    let mut total_without = 0.0f64;
    for frame in 0..FRAMES {
        for (use_cards, total) in [(true, &mut total_with), (false, &mut total_without)] {
            let start = Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            if use_cards {
                lighter.record(
                    &device,
                    &queue,
                    &mut encoder,
                    &atlas,
                    &CardLightInputs {
                        sdf: &gpu_sdf,
                        prev_grid: &grid,
                        prev_irradiance: &irradiance,
                        probe_offset: &offsets,
                        sun_dir: sun,
                        sun_radiance: Vec3::ONE,
                        points: &[],
                        frame_index: frame,
                    },
                );
            }
            for cascade in 0..2u32 {
                let spacing = if cascade == 0 { 0.5 } else { 2.0 };
                let params = GiTraceParams {
                    grid_origin: Vec3::new(-4.0, 0.0, -4.0),
                    spacing,
                    dims: CASCADE_DIMS,
                    rays: RAYS,
                    frame_index: frame,
                    sun_dir: sun,
                    sun_radiance: Vec3::ONE,
                    scroll: [0; 3],
                    probe_base: cascade * probes_per_cascade,
                };
                let inputs = TraceInputs {
                    sdf: &gpu_sdf,
                    sky: &sky,
                    prev_grid: &grid,
                    prev_irradiance: &irradiance,
                    probe_offset: &offsets,
                    cards: use_cards.then_some(&atlas),
                };
                tracer.record_slot(
                    &device,
                    &queue,
                    &mut encoder,
                    &params,
                    cascade as usize,
                    &inputs,
                );
            }
            queue.submit([encoder.finish()]);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            *total += start.elapsed().as_secs_f64();
        }
    }
    let with = total_with * 1000.0 / f64::from(FRAMES);
    let without = total_without * 1000.0 / f64::from(FRAMES);
    println!("GI wall clock per frame, 2 cascades, {OBJECTS} objects:");
    println!("  with cards (light + trace): {with:.3} ms");
    println!("  probe path only (trace):    {without:.3} ms");
}

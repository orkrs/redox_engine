//! GPU tests for the global SDF upload and the probe ray trace. Each test returns early when
//! no adapter is available.

mod common;

use redox_math::{Aabb, Vec3};
use redox_render::gi::sdf_ref::cornell_box;
use redox_render::gi::trace::{GiTraceParams, GiTracer};
use redox_render::sdf::global::{GLOBAL_SDF_DIMS, GlobalSdf, SdfBox};
use redox_render::sdf::global_gpu::GlobalSdfGpu;

/// Sky colour in 8-bit sRGB-free unorm: (0.2, 0.4, 0.9).
const SKY: [u8; 4] = [51, 102, 230, 255];

fn setup() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = common::adapter()?;
    common::device_with_bc(&adapter)
}

fn cornell_sdf() -> GlobalSdf {
    let scene = cornell_box();
    let bounds = Aabb {
        min: Vec3::new(-2.2, -0.2, -2.2),
        max: Vec3::new(2.2, 4.2, 2.2),
    };
    GlobalSdf::from_boxes(&scene.boxes, bounds, GLOBAL_SDF_DIMS)
}

/// Uniform environment cube with a single colour on every face.
fn sky_cube(device: &wgpu::Device, queue: &wgpu::Queue, colour: [u8; 4]) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gi_test_sky"),
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
    let data: Vec<u8> = colour.iter().copied().cycle().take(4 * 4 * 6 * 4).collect();
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

fn read_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Buffer,
    bytes: u64,
) -> Vec<u8> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gi_test_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(src, 0, &staging, 0, bytes);
    queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().expect("map callback").expect("map read");
    let data = slice.get_mapped_range().expect("mapped range").to_vec();
    staging.unmap();
    data
}

/// Runs one trace over `sdf` and returns `[r, g, b, distance]` per ray, probe-major.
fn trace(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sdf: &GlobalSdf,
    sky: &wgpu::TextureView,
    params: GiTraceParams,
) -> Vec<[f32; 4]> {
    let mut gpu_sdf = GlobalSdfGpu::new(device, queue, sdf.dims);
    gpu_sdf.upload(device, queue, sdf);
    let probes: u32 = params.dims.iter().product();
    let tracer = GiTracer::new(device, probes);
    let mut encoder = device.create_command_encoder(&Default::default());
    tracer.record(device, queue, &mut encoder, &params, &gpu_sdf, sky);
    queue.submit(Some(encoder.finish()));
    let bytes = read_bytes(
        device,
        queue,
        &tracer.ray_data,
        u64::from(probes * params.rays) * 16,
    );
    bytes
        .chunks_exact(16)
        .map(|record| {
            let f = |i: usize| {
                f32::from_le_bytes(
                    <[u8; 4]>::try_from(&record[i * 4..i * 4 + 4]).unwrap_or_default(),
                )
            };
            [f(0), f(1), f(2), f(3)]
        })
        .collect()
}

fn probe_at(position: Vec3, rays: u32, frame_index: u32, sun_dir: Vec3) -> GiTraceParams {
    GiTraceParams {
        grid_origin: position,
        spacing: 1.0,
        dims: [1, 1, 1],
        rays,
        frame_index,
        sun_dir,
        sun_radiance: Vec3::ZERO,
        scroll: [0; 3],
        probe_base: 0,
    }
}

#[test]
fn the_global_sdf_gpu_upload_roundtrips() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let sdf = cornell_sdf();
    let mut gpu = GlobalSdfGpu::new(&device, &queue, sdf.dims);
    gpu.upload(&device, &queue, &sdf);

    let [dx, dy, dz] = sdf.dims;
    let bytes_len = u64::from(dx) * u64::from(dy) * u64::from(dz) * 2;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gi_test_distance_readback"),
        size: bytes_len,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &gpu.distance_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dx * 2),
                rows_per_image: Some(dy),
            },
        },
        wgpu::Extent3d {
            width: dx,
            height: dy,
            depth_or_array_layers: dz,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().expect("map callback").expect("map read");
    let data = slice.get_mapped_range().expect("mapped range").to_vec();
    staging.unmap();

    for (x, y, z) in [
        (0, 0, 0),
        (3, 5, 7),
        (64, 2, 100),
        (127, 127, 127),
        (60, 60, 60),
    ] {
        let i = GlobalSdf::index(sdf.dims, x, y, z);
        let bits = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
        let gpu_value = half::f16::from_bits(bits).to_f32();
        assert!(
            (gpu_value - sdf.distance[i]).abs() < 1e-2,
            "texel ({x},{y},{z}): gpu {gpu_value} vs cpu {}",
            sdf.distance[i]
        );
    }
}

#[test]
fn every_ray_from_the_room_centre_hits_a_wall() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let sdf = cornell_sdf();
    let sky = sky_cube(&device, &queue, SKY);
    let rays = trace(
        &device,
        &queue,
        &sdf,
        &sky,
        probe_at(Vec3::new(0.0, 2.0, 0.0), 64, 0, Vec3::ZERO),
    );
    assert_eq!(rays.len(), 64);
    for (i, ray) in rays.iter().enumerate() {
        assert!(ray[3] < 40.0, "ray {i} escaped (distance {})", ray[3]);
    }
}

#[test]
fn rays_towards_the_emitter_return_its_radiance() {
    // Probe just under the ceiling panel: the upward rays land on the emitter.
    let Some((device, queue)) = setup() else {
        return;
    };
    let sdf = cornell_sdf();
    let sky = sky_cube(&device, &queue, SKY);
    let rays = trace(
        &device,
        &queue,
        &sdf,
        &sky,
        probe_at(Vec3::new(0.0, 3.5, 0.0), 64, 0, Vec3::ZERO),
    );
    let brightest = rays.iter().map(|r| r[0]).fold(0.0f32, f32::max);
    assert!(brightest > 1.0, "no ray reached the emitter");
    assert!(
        (brightest - 12.0).abs() < 0.6,
        "emitter radiance {brightest}, expected 12 +-5%"
    );
}

#[test]
fn the_sky_is_returned_for_rays_that_escape() {
    // Open scene: a floor only, so upward rays leave the volume unobstructed.
    let Some((device, queue)) = setup() else {
        return;
    };
    let floor = SdfBox {
        min: Vec3::new(-20.0, -0.2, -20.0),
        max: Vec3::new(20.0, 0.0, 20.0),
        albedo: Vec3::splat(0.5),
        emissive: Vec3::ZERO,
    };
    let bounds = Aabb {
        min: Vec3::splat(-20.0),
        max: Vec3::splat(20.0),
    };
    let sdf = GlobalSdf::from_boxes(&[floor], bounds, [64, 64, 64]);
    let sky = sky_cube(&device, &queue, SKY);
    let rays = trace(
        &device,
        &queue,
        &sdf,
        &sky,
        // Well above the floor: a probe within half a voxel (0.31 m) of it counts as inside.
        probe_at(Vec3::new(0.0, 3.0, 0.0), 64, 0, Vec3::ZERO),
    );
    // Misses report the trace limit (40 m) as their distance.
    let escaped: Vec<_> = rays.iter().filter(|r| r[3] >= 39.0).collect();
    assert!(escaped.len() >= 8, "only {} rays escaped", escaped.len());
    let expected = SKY.map(|c| f32::from(c) / 255.0);
    for ray in escaped {
        for c in 0..3 {
            assert!(
                (ray[c] - expected[c]).abs() < 0.02,
                "escaped ray radiance {ray:?} vs sky {expected:?}"
            );
        }
    }
}

#[test]
fn the_ray_directions_change_between_frames() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let sdf = cornell_sdf();
    let sky = sky_cube(&device, &queue, SKY);
    let frame0 = trace(
        &device,
        &queue,
        &sdf,
        &sky,
        probe_at(Vec3::new(0.0, 2.0, 0.0), 64, 0, Vec3::ZERO),
    );
    let frame1 = trace(
        &device,
        &queue,
        &sdf,
        &sky,
        probe_at(Vec3::new(0.0, 2.0, 0.0), 64, 1, Vec3::ZERO),
    );
    let differs = frame0
        .iter()
        .zip(&frame1)
        .any(|(a, b)| (a[3] - b[3]).abs() > 1e-3);
    assert!(differs, "frame 0 and frame 1 traced identical directions");
}

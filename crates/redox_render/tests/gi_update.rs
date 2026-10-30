//! Probe atlas tests (GI-01 step 4). The CPU tests run everywhere; the GPU tests return early
//! when no adapter is available.

mod common;

use redox_math::Vec3;
use redox_render::gi::atlas::{AtlasLayout, AtlasUpdate, ProbeAtlas, probes_per_row};
use redox_render::gi::oct::{border_source, fibonacci_sphere};

/// Rays per probe used by the GPU tests.
const RAYS: u32 = 64;

fn setup() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = common::adapter()?;
    common::device_with_bc(&adapter)
}

#[test]
fn probes_per_row_is_the_ceiling_square_root() {
    assert_eq!(probes_per_row(1), 1);
    assert_eq!(probes_per_row(5), 3);
    assert_eq!(probes_per_row(16), 4);
    assert_eq!(probes_per_row(17), 5);
    let layout = AtlasLayout::new(5, 8);
    assert_eq!(layout.size(), [30, 20]);
    assert_eq!(layout.texel_origin(4), [10, 10]);
}

#[test]
fn probe_uv_stays_inside_the_probe_tile() {
    let layout = AtlasLayout::new(5, 8);
    let size = layout.size();
    for probe in 0..5 {
        let origin = layout.texel_origin(probe);
        for dir in fibonacci_sphere_points(200) {
            let uv = layout.uv_for(probe, dir);
            let x = uv[0] * size[0] as f32;
            let y = uv[1] * size[1] as f32;
            let lo = [origin[0] as f32 + 1.0, origin[1] as f32 + 1.0];
            let hi = [lo[0] + 8.0, lo[1] + 8.0];
            assert!(x >= lo[0] - 1e-3 && x <= hi[0] + 1e-3, "x {x} outside tile of probe {probe}");
            assert!(y >= lo[1] - 1e-3 && y <= hi[1] + 1e-3, "y {y} outside tile of probe {probe}");
        }
    }
}

fn fibonacci_sphere_points(count: u32) -> impl Iterator<Item = Vec3> {
    (0..count).map(move |i| fibonacci_sphere(i, count))
}

#[test]
fn border_texels_copy_an_inner_texel_and_inner_texels_are_fixed() {
    let inner = 8;
    for x in 1..=inner {
        for y in 1..=inner {
            assert_eq!(border_source([x, y], inner), [x, y]);
        }
    }
    for k in 0..inner + 2 {
        for ring in [[0, k], [inner + 1, k], [k, 0], [k, inner + 1]] {
            let src = border_source(ring, inner);
            assert!((1..=inner).contains(&src[0]) && (1..=inner).contains(&src[1]));
        }
    }
}

#[test]
fn a_corner_border_texel_copies_the_opposite_inner_corner() {
    assert_eq!(border_source([0, 0], 8), [8, 8]);
    assert_eq!(border_source([9, 9], 8), [1, 1]);
}

#[test]
fn a_uniform_sky_converges_to_a_uniform_irradiance() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let mut atlas = ProbeAtlas::new(&device, 1);
    let rays = vec![[1.0, 1.0, 1.0, 40.0]; RAYS as usize];
    for frame in 0..100 {
        run(&device, &queue, &mut atlas, &rays, frame, 0.9);
    }
    let layout = atlas.irradiance_layout();
    let pixels = read_atlas(&device, &queue, atlas.irradiance(), layout.size());
    let width = layout.size()[0];
    for y in 1..=8 {
        for x in 1..=8 {
            let texel = pixels[(y * width + x) as usize];
            // Stored as acc^(1/5); decode and compare with pi for unit radiance.
            let decoded = texel[0].powf(5.0);
            assert!((decoded / std::f32::consts::PI - 1.0).abs() < 0.03, "{decoded}");
        }
    }
}

#[test]
fn the_hysteresis_blends_old_and_new_values() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let mut atlas = ProbeAtlas::new(&device, 1);
    run(&device, &queue, &mut atlas, &vec![[0.0, 0.0, 0.0, 1.0]; RAYS as usize], 0, 0.0);
    let layout = atlas.visibility_layout();
    let before = read_atlas(&device, &queue, atlas.visibility(), layout.size());
    assert!((before[(layout.size()[0] + 1) as usize][0] - 1.0).abs() < 0.01);
    run(&device, &queue, &mut atlas, &vec![[0.0, 0.0, 0.0, 0.0]; RAYS as usize], 1, 0.97);
    let after = read_atlas(&device, &queue, atlas.visibility(), layout.size());
    let value = after[(layout.size()[0] + 1) as usize][0];
    assert!((value - 0.97).abs() < 0.01, "{value}");
}

#[test]
fn borders_replicate_the_opposite_edge() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let mut atlas = ProbeAtlas::new(&device, 1);
    let rays: Vec<[f32; 4]> = (0..RAYS)
        .map(|r| [(r % 7) as f32 / 7.0, (r % 5) as f32 / 5.0, (r % 3) as f32 / 3.0, 5.0])
        .collect();
    run(&device, &queue, &mut atlas, &rays, 0, 0.0);
    let layout = atlas.irradiance_layout();
    let width = layout.size()[0];
    let pixels = read_atlas(&device, &queue, atlas.irradiance(), layout.size());
    for y in 0..10u32 {
        for x in 0..10u32 {
            if x != 0 && y != 0 && x != 9 && y != 9 {
                continue;
            }
            let src = border_source([x, y], 8);
            let got = pixels[(y * width + x) as usize];
            let want = pixels[(src[1] * width + src[0]) as usize];
            for c in 0..3 {
                assert!((got[c] - want[c]).abs() < 1e-3, "texel {x},{y} channel {c}");
            }
        }
    }
}

#[test]
fn the_visibility_texture_stores_the_mean_distance() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let mut atlas = ProbeAtlas::new(&device, 1);
    run(&device, &queue, &mut atlas, &vec![[0.0, 0.0, 0.0, 3.0]; RAYS as usize], 0, 0.0);
    let layout = atlas.visibility_layout();
    let pixels = read_atlas(&device, &queue, atlas.visibility(), layout.size());
    let texel = pixels[(layout.size()[0] + 1) as usize];
    assert!((texel[0] - 3.0).abs() < 0.01, "mean {}", texel[0]);
    assert!((texel[1] - 9.0).abs() < 0.05, "mean square {}", texel[1]);
}

/// Records one update over `rays` and waits for it to finish.
fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &mut ProbeAtlas,
    rays: &[[f32; 4]],
    frame_index: u32,
    hysteresis: f32,
) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gi_update_test_rays"),
        size: (rays.len() * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytemuck::cast_slice(rays));
    let mut encoder = device.create_command_encoder(&Default::default());
    let update = AtlasUpdate {
        rays: RAYS,
        frame_index,
        hysteresis,
        max_probe_distance: 8.0,
    };
    atlas.record_update(device, queue, &mut encoder, &update, &buffer);
    queue.submit(Some(encoder.finish()));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

/// Reads an `Rgba16Float` atlas back as row-major `[f32; 4]` texels.
fn read_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: [u32; 2],
) -> Vec<[f32; 4]> {
    let padded = (size[0] * 8).next_multiple_of(256);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gi_update_readback"),
        size: u64::from(padded) * u64::from(size[1]),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
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
    let mut texels = Vec::with_capacity((size[0] * size[1]) as usize);
    for y in 0..size[1] {
        let row = &data[(y * padded) as usize..];
        for x in 0..size[0] {
            let px = &row[(x * 8) as usize..(x * 8 + 8) as usize];
            texels.push(std::array::from_fn(|c| {
                half::f16::from_le_bytes([px[c * 2], px[c * 2 + 1]]).to_f32()
            }));
        }
    }
    texels
}

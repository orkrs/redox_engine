//! Surface card lighting (GI-02 step 3). The GPU tests return early when no adapter exists.

mod common;

use redox_math::{Aabb, Mat4, Vec3};
use redox_render::gi::card_atlas::{CARD_TEXEL_BYTES, CardAtlas};
use redox_render::gi::card_capture::{CardMaterial, ObjectCards, capture_object};
use redox_render::gi::card_light::{CardLightInputs, CardLighter, CardPointGpu};
use redox_render::gi::cards::CardFace;
use redox_render::gi::grid_gpu::GiGridGpu;
use redox_render::mesh::{Mesh, Vertex};
use redox_render::sdf::global::{GlobalSdf, SdfBox};
use redox_render::sdf::global_gpu::GlobalSdfGpu;

/// Atlas edge for the GPU tests (rows of 256 texels keep the readback alignment).
const TEST_ATLAS: u32 = 256;
/// Voxels per axis of the test SDF.
const SDF_DIMS: [u32; 3] = [48, 48, 48];

fn setup() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = common::adapter()?;
    common::device_with_bc(&adapter)
}

/// Axis-aligned cube from `-half` to `half` with outward normals.
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

fn cube_cards(half: f32, albedo: Vec3, emissive: Vec3, texel: f32) -> ObjectCards {
    capture_object(
        &cube_mesh(half),
        &Mat4::IDENTITY,
        &CardMaterial { albedo, emissive },
        texel,
    )
}

/// Everything the light pass reads, built once per test scene.
struct Scene {
    sdf: GlobalSdfGpu,
    grid: wgpu::Buffer,
    irradiance: wgpu::TextureView,
    probe_offset: wgpu::Buffer,
    lighter: CardLighter,
    atlas: CardAtlas,
}

fn scene(device: &wgpu::Device, queue: &wgpu::Queue, boxes: &[SdfBox], cards: &[(ObjectCards, u32)]) -> Scene {
    let bounds = Aabb {
        min: Vec3::splat(-6.0),
        max: Vec3::splat(6.0),
    };
    let mut sdf = GlobalSdfGpu::new(device, queue, SDF_DIMS);
    sdf.upload(device, queue, &GlobalSdf::from_boxes(boxes, bounds, SDF_DIMS));
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
    let probe_offset = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_probe_offset"),
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut atlas = CardAtlas::new(device, TEST_ATLAS);
    atlas.set_objects(device, queue, cards);
    Scene {
        sdf,
        grid,
        irradiance,
        probe_offset,
        lighter: CardLighter::new(device),
        atlas,
    }
}

/// Lights the scene once with the given sun and points at `frame_index`, and waits for it.
fn light(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    s: &Scene,
    sun_dir: Vec3,
    sun_radiance: Vec3,
    points: &[CardPointGpu],
    frame_index: u32,
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    s.lighter.record(
        device,
        queue,
        &mut encoder,
        &s.atlas,
        &CardLightInputs {
            sdf: &s.sdf,
            prev_grid: &s.grid,
            prev_irradiance: &s.irradiance,
            probe_offset: &s.probe_offset,
            sun_dir,
            sun_radiance,
            points,
            frame_index,
        },
    );
    queue.submit([encoder.finish()]);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

/// Reads the radiance atlas back as linear RGBA (f16 decoded).
fn read_radiance(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &CardAtlas) -> Vec<[f32; 4]> {
    let row = atlas.size * 8;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_radiance_readback"),
        size: u64::from(row) * u64::from(atlas.size),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &atlas.radiance_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(atlas.size),
            },
        },
        wgpu::Extent3d {
            width: atlas.size,
            height: atlas.size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().expect("map callback").expect("map");
    let data = slice.get_mapped_range().expect("mapped range");
    let texels = data
        .chunks_exact(8)
        .map(|c| {
            let h = |i: usize| half::f16::from_bits(u16::from_le_bytes([c[i], c[i + 1]])).to_f32();
            [h(0), h(2), h(4), h(6)]
        })
        .collect();
    drop(data);
    buffer.unmap();
    texels
}

/// Radiance of the centre texel of `face` of the object whose face rectangles are `rects`.
fn centre_radiance(rgba: &[[f32; 4]], size: u32, rects: &[[u32; 4]; 6], face: CardFace) -> [f32; 4] {
    let [x, y, w, h] = rects[face.index()];
    rgba[((x + w / 2) + (y + h / 2) * size) as usize]
}

/// Face rectangles of object `id` as the upload packs them (same packer, same order).
fn rects_of(cards: &[(ObjectCards, u32)]) -> [[u32; 4]; 6] {
    redox_render::gi::card_atlas::pack_objects(cards, TEST_ATLAS).table[0].rects
}

/// Lights every row of the atlas: four consecutive frames cover all rows modulo 4.
fn light_all_rows(device: &wgpu::Device, queue: &wgpu::Queue, s: &Scene, sun_dir: Vec3, sun_rad: Vec3, points: &[CardPointGpu]) {
    for frame in 0..4 {
        light(device, queue, s, sun_dir, sun_rad, points, frame);
    }
}

#[test]
fn a_lit_card_is_brighter_than_a_card_facing_away_from_the_sun() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let cards = [(cube_cards(1.0, Vec3::splat(0.8), Vec3::ZERO, 0.1), 0)];
    let s = scene(&device, &queue, &[], &cards);
    light_all_rows(&device, &queue, &s, Vec3::X, Vec3::splat(1.0), &[]);
    let rgba = read_radiance(&device, &queue, &s.atlas);
    let rects = rects_of(&cards);
    let lit = centre_radiance(&rgba, TEST_ATLAS, &rects, CardFace::PosX);
    let dark = centre_radiance(&rgba, TEST_ATLAS, &rects, CardFace::NegX);
    assert!(lit[0] > 0.0, "lit face is black");
    assert!(lit[0] > 5.0 * dark[0], "lit {lit:?} dark {dark:?}");
}

#[test]
fn a_card_in_shadow_receives_no_sun() {
    let Some((device, queue)) = setup() else {
        return;
    };
    // A wall between the sun (+X) and the cube, 1 m from the cube's +X face.
    let wall = SdfBox {
        min: Vec3::new(2.0, -4.0, -4.0),
        max: Vec3::new(2.4, 4.0, 4.0),
        albedo: Vec3::ONE,
        emissive: Vec3::ZERO,
    };
    let cards = [(cube_cards(1.0, Vec3::splat(0.8), Vec3::ZERO, 0.1), 0)];
    let s = scene(&device, &queue, &[wall], &cards);
    light_all_rows(&device, &queue, &s, Vec3::X, Vec3::splat(1.0), &[]);
    let rgba = read_radiance(&device, &queue, &s.atlas);
    let lit = centre_radiance(&rgba, TEST_ATLAS, &rects_of(&cards), CardFace::PosX);
    // No previous-frame irradiance is bound in this test, so only direct light could remain.
    assert!(lit[0] < 0.01, "shadowed face {lit:?}");
}

#[test]
fn a_point_light_brightens_the_near_card() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let cards = [(cube_cards(1.0, Vec3::splat(0.8), Vec3::ZERO, 0.1), 0)];
    let rects = rects_of(&cards);
    let s = scene(&device, &queue, &[], &cards);
    light_all_rows(&device, &queue, &s, Vec3::ZERO, Vec3::ZERO, &[]);
    let dark = centre_radiance(&read_radiance(&device, &queue, &s.atlas), TEST_ATLAS, &rects, CardFace::PosX);
    let point = CardPointGpu {
        pos_intensity: [2.5, 0.0, 0.0, 4.0],
        color_radius: [1.0, 1.0, 1.0, 10.0],
    };
    light_all_rows(&device, &queue, &s, Vec3::ZERO, Vec3::ZERO, &[point]);
    let bright = centre_radiance(&read_radiance(&device, &queue, &s.atlas), TEST_ATLAS, &rects, CardFace::PosX);
    assert!(bright[0] > dark[0] + 0.01, "point light: {bright:?} vs {dark:?}");
}

#[test]
fn an_emissive_card_keeps_its_emission() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let cards = [(cube_cards(1.0, Vec3::ZERO, Vec3::splat(5.0), 0.1), 0)];
    let s = scene(&device, &queue, &[], &cards);
    light_all_rows(&device, &queue, &s, Vec3::ZERO, Vec3::ZERO, &[]);
    let rgba = read_radiance(&device, &queue, &s.atlas);
    let e = centre_radiance(&rgba, TEST_ATLAS, &rects_of(&cards), CardFace::PosX);
    // Emission scale 16 stored in 8 bits: 5 becomes 80/255 * 16 = 5.02, within 2 %.
    assert!((e[0] - 5.0).abs() <= 0.02 * 5.0, "emission {e:?}");
}

#[test]
fn only_a_quarter_of_the_atlas_is_updated_per_frame() {
    let Some((device, queue)) = setup() else {
        return;
    };
    let cards = [(cube_cards(1.0, Vec3::splat(0.5), Vec3::splat(1.0), 0.1), 0)];
    let s = scene(&device, &queue, &[], &cards);
    let lit_texels = |rgba: &[[f32; 4]]| rgba.iter().filter(|t| t[3] > 0.5).count() as f32;
    light(&device, &queue, &s, Vec3::ZERO, Vec3::ZERO, &[], 0);
    let after_one = lit_texels(&read_radiance(&device, &queue, &s.atlas));
    for frame in 1..4 {
        light(&device, &queue, &s, Vec3::ZERO, Vec3::ZERO, &[], frame);
    }
    let after_four = lit_texels(&read_radiance(&device, &queue, &s.atlas));
    assert!(after_four > 0.0);
    let fraction = after_one / after_four;
    assert!((0.2..=0.3).contains(&fraction), "one frame lit {fraction:.3} of the atlas");
}

#[test]
fn the_card_atlas_is_counted_in_the_memory_report() {
    // 2048² texels at 16 bytes (albedo 4, normal/depth 4, radiance 8) is 64 MiB.
    let bytes = u64::from(redox_render::gi::card_atlas::CARD_ATLAS_SIZE).pow(2) * CARD_TEXEL_BYTES;
    assert_eq!(bytes, 64 * 1024 * 1024);
}

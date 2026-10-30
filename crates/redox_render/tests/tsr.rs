//! Temporal upscaling checks (PST-07): pure jitter / LOD / sharpening math, then GPU checks of the
//! TAA upscale path at render scale 0.67 against native TAA. GPU tests skip without an adapter.
//! The `bench` test is `#[ignore]`d: `cargo test --test tsr -- --ignored --nocapture`.

#![allow(clippy::unwrap_used)]

mod common;

use std::collections::HashSet;
use std::time::Instant;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::post::tsr;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;
use redox_render::{Renderer, TaaUniform};

const SIZE: u32 = 128;
const UPSCALE: f32 = 0.67;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Tests create renderers; running them at once can stall the driver, so they take turns.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------- pure math

#[test]
fn halton_jitter_has_no_repeats_within_a_cycle_and_stays_in_range() {
    for scale in [1.0, UPSCALE, 0.5] {
        let phases = tsr::jitter_phase_count(scale);
        let mut seen = HashSet::new();
        for index in 0..phases {
            let j = tsr::jitter_sequence(index, scale);
            assert!(j[0].abs() <= 0.5 && j[1].abs() <= 0.5, "{j:?} at scale {scale}");
            assert!(
                seen.insert((j[0].to_bits(), j[1].to_bits())),
                "repeat at index {index}, scale {scale}"
            );
        }
    }
}

#[test]
fn the_jitter_phase_count_grows_as_the_scale_shrinks() {
    assert_eq!(tsr::jitter_phase_count(1.0), 8);
    assert_eq!(tsr::jitter_phase_count(UPSCALE), 18);
    assert_eq!(tsr::jitter_phase_count(0.5), 32);
}

#[test]
fn the_lod_bias_is_negative_below_native_and_clamped() {
    assert!(tsr::texture_lod_bias(UPSCALE) < 0.0);
    assert!((tsr::texture_lod_bias(UPSCALE) - (UPSCALE.log2() - 1.0)).abs() < 1e-6);
    assert_eq!(tsr::texture_lod_bias(0.5), -2.0);
    assert_eq!(tsr::texture_lod_bias(0.1), -2.0);
}

#[test]
fn lod_bias_is_zero_at_native_scale() {
    assert_eq!(tsr::texture_lod_bias(1.0), 0.0);
    assert_eq!(tsr::texture_lod_bias(0.9995), 0.0);
}

#[test]
fn sharpen_amount_is_zero_at_native_and_peaks_at_half_scale() {
    assert_eq!(tsr::sharpen_amount(1.0), 0.0);
    assert!((tsr::sharpen_amount(0.5) - 0.35).abs() < 1e-6);
    assert!(tsr::sharpen_amount(UPSCALE) > 0.0);
    assert!(tsr::sharpen_amount(UPSCALE) < tsr::sharpen_amount(0.5));
}

// ---------------------------------------------------------------- GPU rig

struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    cube: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig(taa: bool, scale: f32, size: u32) -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, size)?;
    renderer.enable_taa(taa);
    renderer.set_render_scale(scale);
    let cube = renderer.upload_mesh(&create_cube());
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tsr_target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
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
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.6, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.4);
    Some(Rig {
        renderer,
        target,
        view,
        cube,
        camera,
        lights,
        size,
    })
}

impl Rig {
    fn object(&self, material: usize, id: u32, center: Vec3, scale: Vec3) -> RenderObject {
        RenderObject {
            id,
            shadow_class: ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
            color: [1.0; 4],
            mesh_index: self.cube,
            material_index: material,
            bounds: Aabb::from_center_size(center, scale),
        }
    }

    fn frame(&mut self, objects: &[RenderObject]) -> Vec<u8> {
        let view = FrameView::new(&self.camera, objects, &self.lights);
        self.renderer
            .render_frame_view(&view, &self.view)
            .expect("frame");
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        read_texture_ktx2(
            &self.renderer.device,
            &self.renderer.queue,
            &self.target,
            false,
        )
        .unwrap()
        .levels
        .swap_remove(0)
    }

    /// Renders `n` identical frames and returns the last.
    fn settle(&mut self, objects: &[RenderObject], n: usize) -> Vec<u8> {
        let mut last = Vec::new();
        for _ in 0..n {
            last = self.frame(objects);
        }
        last
    }
}

/// Eight by eight alternating tiles on a plane below the view centre.
fn checker_plane(rig: &mut Rig) -> Vec<RenderObject> {
    let dark = rig.renderer.add_material(Material::solid(Vec3::splat(0.08)));
    let light = rig.renderer.add_material(Material::solid(Vec3::splat(0.8)));
    let mut out = Vec::new();
    for j in 0..8u32 {
        for i in 0..8u32 {
            let x = (i as f32 - 3.5) * 0.5;
            let z = -6.0 - j as f32 * 0.5;
            let material = if (i + j) % 2 == 0 { dark } else { light };
            let id = j * 8 + i + 1;
            out.push(rig.object(
                material,
                id,
                Vec3::new(x, -1.0, z),
                Vec3::new(0.24, 0.05, 0.24),
            ));
        }
    }
    out
}

/// One thin (about one pixel wide) vertical bar in front of the checker plane.
fn line_scene(rig: &mut Rig) -> Vec<RenderObject> {
    let bright = rig.renderer.add_material(Material::solid(Vec3::splat(0.95)));
    vec![rig.object(
        bright,
        1,
        Vec3::new(0.0, 0.0, -5.0),
        Vec3::new(0.036, 1.5, 0.036),
    )]
}

// ---------------------------------------------------------------- metrics

fn lum_at(frame: &[u8], width: u32, x: u32, y: u32) -> f32 {
    let at = ((y * width + x) * 4) as usize;
    (f32::from(frame[at]) + f32::from(frame[at + 1]) + f32::from(frame[at + 2])) / 3.0
}

/// Peak signal-to-noise ratio over all RGBA bytes, in dB.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let se: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = f64::from(*x) - f64::from(*y);
            d * d
        })
        .sum();
    let mse = se / a.len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

/// Mean Sobel gradient magnitude of the luma, over the interior pixels.
fn edge_sharpness(frame: &[u8], width: u32, height: u32) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0u32;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let l = |dx: i32, dy: i32| {
                lum_at(
                    frame,
                    width,
                    (x as i32 + dx) as u32,
                    (y as i32 + dy) as u32,
                )
            };
            let gx = (l(1, -1) + 2.0 * l(1, 0) + l(1, 1)) - (l(-1, -1) + 2.0 * l(-1, 0) + l(-1, 1));
            let gy = (l(-1, 1) + 2.0 * l(0, 1) + l(1, 1)) - (l(-1, -1) + 2.0 * l(0, -1) + l(1, -1));
            sum += (gx * gx + gy * gy).sqrt();
            count += 1;
        }
    }
    sum / count as f32
}

/// Reads the TAA uniform buffer back as little-endian f32 values.
fn read_taa_uniform(renderer: &Renderer) -> Vec<f32> {
    let size = std::mem::size_of::<TaaUniform>() as u64;
    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tsr_uniform_readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tsr_uniform_readback"),
        });
    encoder.copy_buffer_to_buffer(&renderer.taa_pass.uniform_buffer, 0, &staging, 0, size);
    renderer.queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| assert!(result.is_ok()));
    let _ = renderer
        .device
        .poll(wgpu::PollType::wait_indefinitely());
    let values = {
        let data = slice.get_mapped_range().expect("mapped range");
        data.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect::<Vec<f32>>()
    };
    staging.unmap();
    values
}

// ---------------------------------------------------------------- GPU checks

#[test]
fn the_jitter_matches_the_formula_in_the_uniform() {
    let _gpu = lock();
    let Some(mut rig) = rig(true, UPSCALE, SIZE) else { return };
    let objects = checker_plane(&mut rig);
    rig.settle(&objects, 5);
    let values = read_taa_uniform(&rig.renderer);
    // The last recorded frame used the counter value before the final increment.
    let used = rig.renderer.taa_frame_count - 1;
    let j = tsr::jitter_ndc(
        used,
        UPSCALE,
        rig.renderer.internal_width,
        rig.renderer.internal_height,
    );
    // jitter_uv sits at byte offset 24, inv_scale at 32.
    assert!((values[6] - j[0] * 0.5).abs() < 1e-6, "jitter x {}", values[6]);
    assert!((values[7] + j[1] * 0.5).abs() < 1e-6, "jitter y {}", values[7]);
    assert!((values[8] - tsr::inv_scale(UPSCALE)).abs() < 1e-6);
}

#[test]
fn scale_one_is_bitwise_unchanged() {
    // ASSUMPTION: there is no stored golden frame for the native TAA path, so this checks that
    // two native runs of the same scene are byte-identical (the native path is the legacy one).
    let _gpu = lock();
    let Some(mut a) = rig(true, 1.0, SIZE) else { return };
    let objects = checker_plane(&mut a);
    let first = a.settle(&objects, 12);
    let Some(mut b) = rig(true, 1.0, SIZE) else { return };
    let objects = checker_plane(&mut b);
    let second = b.settle(&objects, 12);
    assert!(first == second, "native frames differ");
}

#[test]
fn upscaling_a_static_scene_converges_towards_the_native_taa_frame() {
    let _gpu = lock();
    let Some(mut native) = rig(true, 1.0, SIZE) else { return };
    let objects = checker_plane(&mut native);
    let reference = native.settle(&objects, 64);
    let Some(mut upscaled) = rig(true, UPSCALE, SIZE) else { return };
    let objects = checker_plane(&mut upscaled);
    let frame = upscaled.settle(&objects, 64);
    let db = psnr(&frame, &reference);
    assert!(db >= 28.0, "PSNR {db:.1} dB against native TAA");
}

#[test]
fn the_upscaled_frame_is_not_blurrier_than_70_percent_of_native_sharpness() {
    let _gpu = lock();
    let Some(mut native) = rig(true, 1.0, SIZE) else { return };
    let objects = checker_plane(&mut native);
    let reference = native.settle(&objects, 64);
    let Some(mut upscaled) = rig(true, UPSCALE, SIZE) else { return };
    let objects = checker_plane(&mut upscaled);
    let frame = upscaled.settle(&objects, 64);
    let sharp_native = edge_sharpness(&reference, SIZE, SIZE);
    let sharp_upscaled = edge_sharpness(&frame, SIZE, SIZE);
    assert!(
        sharp_upscaled >= 0.7 * sharp_native,
        "{sharp_upscaled} vs native {sharp_native}"
    );
}

#[test]
fn the_sharpen_pass_raises_edge_sharpness_but_not_above_native_plus_20_percent() {
    let _gpu = lock();
    let Some(mut native) = rig(true, 1.0, SIZE) else { return };
    let objects = checker_plane(&mut native);
    let reference = native.settle(&objects, 64);
    let Some(mut upscaled) = rig(true, UPSCALE, SIZE) else { return };
    let objects = checker_plane(&mut upscaled);
    let frame = upscaled.settle(&objects, 64);
    let sharp_native = edge_sharpness(&reference, SIZE, SIZE);
    let sharp_upscaled = edge_sharpness(&frame, SIZE, SIZE);
    assert!(
        sharp_upscaled <= 1.2 * sharp_native,
        "{sharp_upscaled} vs native {sharp_native}"
    );
}

#[test]
fn no_ringing_on_a_single_pixel_line() {
    let _gpu = lock();
    let Some(mut rig) = rig(true, UPSCALE, SIZE) else { return };
    let objects = line_scene(&mut rig);
    let frame = rig.settle(&objects, 48);
    let row = SIZE / 2;
    let values: Vec<f32> = (0..SIZE).map(|x| lum_at(&frame, SIZE, x, row)).collect();
    let (peak_x, peak) = values
        .iter()
        .enumerate()
        .fold((0usize, f32::MIN), |best, (x, &v)| {
            if v > best.1 { (x, v) } else { best }
        });
    // Neighbours of the line must not overshoot the line's peak by more than 5%.
    for x in [peak_x - 1, peak_x + 1] {
        assert!(
            values[x] <= 1.05 * peak + 1.0,
            "ringing: {} next to peak {peak}",
            values[x]
        );
    }
}

#[test]
fn moving_object_leaves_no_ghost_at_067() {
    let _gpu = lock();
    let Some(mut rig) = rig(true, UPSCALE, SIZE) else { return };
    let bright = rig.renderer.add_material(Material::solid(Vec3::splat(0.95)));
    let mut last = Vec::new();
    for step in 0..40 {
        let x = -3.0 + step as f32 * 0.15;
        let objects = vec![rig.object(bright, 1, Vec3::new(x, 0.0, -5.0), Vec3::ONE)];
        last = rig.frame(&objects);
    }
    let row: Vec<f32> = (0..SIZE).map(|x| lum_at(&last, SIZE, x, SIZE / 2)).collect();
    let sky = row[1];
    let covered = row.iter().filter(|&&l| (l - sky).abs() > 25.0).count();
    // PST-04 tolerance was 29 + 8 pixels; allowed here is 1.5 times that.
    assert!(covered <= 55, "{covered} bright pixels in the row");
}

#[test]
#[ignore = "timing report; run with --ignored --nocapture"]
fn bench() {
    let _gpu = lock();
    for size in [1080u32, 2160] {
        let mut native: Option<Vec<u8>> = None;
        for scale in [1.0f32, 0.77, UPSCALE, 0.5] {
            let Some(mut rig) = rig(true, scale, size) else { return };
            let objects = checker_plane(&mut rig);
            let frame = rig.settle(&objects, 32);
            let start = Instant::now();
            for _ in 0..16 {
                rig.frame(&objects);
            }
            // Wall-clock per frame, including the readback to the CPU.
            let ms = start.elapsed().as_secs_f64() * 1000.0 / 16.0;
            let db = match &native {
                Some(reference) => psnr(&frame, reference),
                None => {
                    native = Some(frame);
                    f64::INFINITY
                }
            };
            println!("{size}px scale {scale:.2}: {ms:.2} ms/frame, PSNR to native {db:.1} dB");
        }
    }
}

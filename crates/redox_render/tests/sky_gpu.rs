//! GPU checks of the procedural atmosphere (SKY-02). Skipped without a GPU.
//! Not run during SKY-02 by the executor (coordinator's rule); run one at a time:
//!   cargo test -p redox_render --test sky_gpu -- --test-threads=1
//! Frame tests with a renderer live in `sky_render_gpu.rs`.

#![allow(clippy::unwrap_used)]

mod common;

use redox_render::resource::ibl::{IBLProcessor, read_texture_ktx2};
use redox_render::sky::{
    AtmosphereParams, SKY_CUBE_SIZE, SKY_VIEW_SIZE, SkyAtmosphere, SkyFrame, TRANSMITTANCE_SIZE,
    transmittance_reference, transmittance_uv_to_rmu,
};

const CAMERA_UNIFORM_BYTES: u64 = 464;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Reads an Rgba16Float texture (mip 0) as f32 texels, row-major, top row first.
fn read_rgba16(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture) -> Vec<[f32; 4]> {
    let ktx = read_texture_ktx2(device, queue, tex, false).expect("readback");
    ktx.levels[0]
        .chunks_exact(8)
        .map(|c| {
            std::array::from_fn(|i| half::f16::from_le_bytes([c[2 * i], c[2 * i + 1]]).to_f32())
        })
        .collect()
}

/// Sun direction (unit, y up) at `elevation_deg` above the horizon, azimuth +X.
fn sun_at(elevation_deg: f32) -> [f32; 3] {
    let e = elevation_deg.to_radians();
    [e.cos(), e.sin(), 0.0]
}

fn sky_frame(elevation_deg: f32) -> SkyFrame {
    SkyFrame {
        sun_dir: sun_at(elevation_deg),
        sun_irradiance: [3.0, 3.0, 3.0],
        camera_altitude_km: 0.0016,
    }
}

/// Device, queue and a procedural sky with its LUTs baked.
fn sky_setup() -> Option<(wgpu::Device, wgpu::Queue, SkyAtmosphere)> {
    let adapter = common::adapter()?;
    let (device, queue) = common::device_with_bc(&adapter)?;
    let camera = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test_camera"),
        size: CAMERA_UNIFORM_BYTES,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sky = SkyAtmosphere::new(
        &device,
        &queue,
        AtmosphereParams::default(),
        &camera,
        HDR_FORMAT,
        1,
    );
    Some((device, queue, sky))
}

/// Updates the sky-view LUT for `frame` and submits it.
fn bake_sky_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sky: &mut SkyAtmosphere,
    frame: SkyFrame,
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    sky.prepare(queue, &mut encoder, frame);
    queue.submit(Some(encoder.finish()));
}

fn texel(px: &[[f32; 4]], width: u32, x: u32, y: u32) -> [f32; 4] {
    px[(y * width + x) as usize]
}

#[test]
fn transmittance_lut_matches_the_cpu_reference() {
    let _g = lock();
    let Some((device, queue, sky)) = sky_setup() else {
        return;
    };
    let (w, h) = TRANSMITTANCE_SIZE;
    let px = read_rgba16(&device, &queue, sky.transmittance_lut());
    let params = *sky.params();
    let mut worst = 0.0f32;
    for k in 0..64u32 {
        let x = (k * 37 + 11) % w;
        let y = (k * 17 + 5) % h;
        let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
        let (r, mu) = transmittance_uv_to_rmu(&params, uv);
        let cpu = transmittance_reference(&params, r - params.planet_radius_km, mu);
        let gpu = texel(&px, w, x, y);
        for c in 0..3 {
            if cpu[c] > 1e-4 {
                worst = worst.max((gpu[c] - cpu[c]).abs() / cpu[c]);
            }
        }
    }
    assert!(worst < 0.02, "transmittance LUT error {worst} exceeds 2 %");
}

#[test]
fn transmittance_is_one_at_zero_distance() {
    let _g = lock();
    let Some((device, queue, sky)) = sky_setup() else {
        return;
    };
    let (w, h) = TRANSMITTANCE_SIZE;
    let px = read_rgba16(&device, &queue, sky.transmittance_lut());
    // Top row of the LUT with the smallest u: straight up from the top of the atmosphere.
    let gpu = texel(&px, w, 0, h - 1);
    for (c, value) in gpu.iter().take(3).enumerate() {
        assert!(
            *value > 0.99,
            "channel {c} transmittance {value} at the top"
        );
    }
    let params = AtmosphereParams::default();
    let cpu = transmittance_reference(&params, params.atmosphere_height_km, 1.0);
    for value in cpu {
        assert!(value > 0.999);
    }
}

#[test]
fn zenith_transmittance_is_greater_than_horizon() {
    let params = AtmosphereParams::default();
    let zenith = transmittance_reference(&params, 0.0, 1.0);
    let horizon = transmittance_reference(&params, 0.0, 0.0);
    for c in 0..3 {
        assert!(
            zenith[c] > horizon[c],
            "channel {c}: zenith {} horizon {}",
            zenith[c],
            horizon[c]
        );
    }
}

#[test]
fn a_low_sun_makes_the_horizon_redder_than_the_zenith() {
    let _g = lock();
    let Some((device, queue, mut sky)) = sky_setup() else {
        return;
    };
    bake_sky_view(&device, &queue, &mut sky, sky_frame(3.0));
    let (w, _) = SKY_VIEW_SIZE;
    let px = read_rgba16(&device, &queue, sky.sky_view_lut());
    // Column 0 is the sun's azimuth. Row 53 sits just below the horizon line (v = 0.5),
    // row 0 is the zenith.
    let horizon = texel(&px, w, 0, 53);
    let zenith = texel(&px, w, 0, 0);
    let ratio_h = horizon[0] / horizon[2].max(1e-9);
    let ratio_z = zenith[0] / zenith[2].max(1e-9);
    assert!(
        ratio_h > ratio_z,
        "R/B at the horizon {ratio_h} must exceed R/B at the zenith {ratio_z}"
    );
}

#[test]
fn the_zenith_is_bluer_at_noon() {
    let _g = lock();
    let Some((device, queue, mut sky)) = sky_setup() else {
        return;
    };
    bake_sky_view(&device, &queue, &mut sky, sky_frame(60.0));
    let (w, _) = SKY_VIEW_SIZE;
    let px = read_rgba16(&device, &queue, sky.sky_view_lut());
    let zenith = texel(&px, w, 0, 0);
    assert!(
        zenith[2] > zenith[0],
        "zenith blue {} must exceed red {} at noon",
        zenith[2],
        zenith[0]
    );
}

#[test]
fn moving_the_sun_by_less_than_one_degree_does_not_rebuild_the_cube() {
    let _g = lock();
    let Some((device, queue, mut sky)) = sky_setup() else {
        return;
    };
    // Frame loop of the renderer: rebuild only when the gate says so.
    for step in 0..10 {
        let frame = sky_frame(30.0 + 0.1 * step as f32);
        if sky.cube_needs_rebuild(frame.sun_dir) {
            let _ = sky.build_cube(&device, &queue, frame);
        }
    }
    assert_eq!(
        sky.cube_builds(),
        1,
        "0.9 degrees of motion must not rebuild"
    );
    let big = sky_frame(31.5);
    assert!(
        sky.cube_needs_rebuild(big.sun_dir),
        "1.5 degrees must rebuild"
    );
}

#[test]
fn ibl_colour_follows_the_sky() {
    let _g = lock();
    let Some((device, queue, mut sky)) = sky_setup() else {
        return;
    };
    let frame = sky_frame(45.0);
    let skybox = sky.build_cube(&device, &queue, frame);
    let processor = IBLProcessor::new(&device);
    let (irradiance, _) = processor.generate_irradiance_map(&device, &queue, &skybox.view);
    let irr = read_texture_ktx2(&device, &queue, &irradiance, true).expect("irradiance");
    let sky_cube = read_texture_ktx2(&device, &queue, &skybox.cubemap, true).expect("sky cube");

    let face_texels = |bytes: &[u8], size: u32, face: usize| -> Vec<[f32; 4]> {
        let n = (size * size) as usize;
        bytes[face * n * 8..(face + 1) * n * 8]
            .chunks_exact(8)
            .map(|c| {
                std::array::from_fn(|i| half::f16::from_le_bytes([c[2 * i], c[2 * i + 1]]).to_f32())
            })
            .collect()
    };
    // +Y is face 2 in layer order. Average the irradiance top face.
    let irr_top = face_texels(&irr.levels[0], 32, 2);
    let mut mean = [0.0f32; 3];
    for t in &irr_top {
        for c in 0..3 {
            mean[c] += t[c] / irr_top.len() as f32;
        }
    }
    // Zenith sky: centre texel of the +Y face of the sky cube.
    let sky_top = face_texels(&sky_cube.levels[0], SKY_CUBE_SIZE, 2);
    let mid = (SKY_CUBE_SIZE / 2 * SKY_CUBE_SIZE + SKY_CUBE_SIZE / 2) as usize;
    let zenith = [sky_top[mid][0], sky_top[mid][1], sky_top[mid][2]];
    let lum = 0.2126 * mean[0] + 0.7152 * mean[1] + 0.0722 * mean[2];
    let grey = [lum, lum, lum];
    let dist = |a: [f32; 3], b: [f32; 3]| (0..3).map(|c| (a[c] - b[c]).powi(2)).sum::<f32>().sqrt();
    assert!(
        dist(mean, zenith) < dist(mean, grey),
        "irradiance {mean:?} should be closer to the zenith {zenith:?} than to grey {grey:?}"
    );
}

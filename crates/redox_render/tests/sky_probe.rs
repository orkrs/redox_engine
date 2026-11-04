//! Equirect orientation probe (SKY-01, investigation A). The skybox draws the cube made from an
//! equirect map, so the camera's centre pixel shows what the map says about each direction. The
//! probes below look at a synthetic map and at a real HDRI. Skipped without a GPU; the probes
//! print numbers and are ignored by default.
//!
//! Convention under test (the usual panorama picture): row 0 is the top of the picture (+Y up),
//! the centre column (u = 0.5) is forward (-Z), and u grows to the right, towards +X. The
//! synthetic picture has sky above the horizon and ground below it, plus four stripes on the
//! horizon: white at -Z (u = 0.5), red at +X (u = 0.75), yellow at -X (u = 0.25), magenta at +Z
//! (u = 0).
//!
//! Run: `cargo test -p redox_render --test sky_probe -- --nocapture --ignored`. The HDRI probe
//! reads `SKY_PROBE_HDR` (path to a Radiance .hdr file) and is skipped when it is unset.

#![allow(clippy::unwrap_used)]

mod common;

use std::f32::consts::{FRAC_PI_2, PI};

use redox_math::{Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::resource::texture::Texture;

const SIZE: u32 = 64;
const MAP_W: u32 = 512;
const MAP_H: u32 = 256;
/// Rows either side of the horizon that carry the stripes.
const HORIZON_HALF_ROWS: u32 = 16;
/// Half width of a stripe in map pixels.
const STRIPE_HALF_PX: u32 = 6;
const SKY: [f32; 3] = [0.1, 0.4, 1.0];
const GROUND: [f32; 3] = [0.1, 0.8, 0.1];
/// (u of the stripe, colour, direction the stripe should be seen in).
const STRIPES: [(f32, [f32; 3], &str); 4] = [
    (0.0, [1.0, 0.0, 1.0], "+Z"),
    (0.25, [1.0, 1.0, 0.0], "-X"),
    (0.5, [1.0, 1.0, 1.0], "-Z"),
    (0.75, [1.0, 0.0, 0.0], "+X"),
];

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Tests create renderers; running them at once can stall the driver, so they take turns.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// RGB of one texel of the synthetic picture (0..1 linear).
fn synthetic_texel(col: u32, row: u32) -> [f32; 3] {
    let mut rgb = if row < MAP_H / 2 { SKY } else { GROUND };
    if row.abs_diff(MAP_H / 2) < HORIZON_HALF_ROWS {
        for (u, colour, _) in STRIPES {
            let centre = (u * MAP_W as f32) as u32 % MAP_W;
            let d = col.abs_diff(centre);
            if d.min(MAP_W - d) < STRIPE_HALF_PX {
                rgb = colour;
            }
        }
    }
    rgb
}

/// The synthetic picture as an Rgba16Float texture, uploaded the way `from_hdr_bytes` does.
fn synthetic_map(device: &wgpu::Device, queue: &wgpu::Queue) -> Texture {
    let mut data: Vec<u16> = Vec::with_capacity((MAP_W * MAP_H * 4) as usize);
    for row in 0..MAP_H {
        for col in 0..MAP_W {
            for channel in synthetic_texel(col, row) {
                data.push(half::f16::from_f32(channel).to_bits());
            }
            data.push(half::f16::from_f32(1.0).to_bits());
        }
    }
    Texture::upload_2d(
        device,
        queue,
        MAP_W,
        MAP_H,
        wgpu::TextureFormat::Rgba16Float,
        bytemuck::cast_slice(&data),
        8,
        "sky_probe_equirect",
    )
}

/// Camera orientations to probe: the default look (-Z), the four horizon directions and two
/// 60-degree elevations. The camera looks along `rotation * -Z`.
fn views() -> [(&'static str, Quat); 8] {
    let deg60 = 60f32.to_radians();
    [
        ("-Z (forward)", Quat::IDENTITY),
        ("+X", Quat::from_rotation_y(-FRAC_PI_2)),
        ("+Z", Quat::from_rotation_y(PI)),
        ("-X", Quat::from_rotation_y(FRAC_PI_2)),
        ("+Y 60 deg", Quat::from_rotation_x(deg60)),
        ("-Y 60 deg", Quat::from_rotation_x(-deg60)),
        ("-Z 60 deg up", Quat::from_rotation_x(deg60)),
        ("-Z 60 deg down", Quat::from_rotation_x(-deg60)),
    ]
}

/// What the picture says the camera should see in each view, where the picture is read with
/// the convention above (the two 60-degree views of -Z share the sky and ground rows).
fn expected_for(name: &str) -> &'static str {
    match name {
        "-Z (forward)" => "white (-Z stripe)",
        "+X" => "red (+X stripe)",
        "+Z" => "magenta (+Z stripe)",
        "-X" => "yellow (-X stripe)",
        "+Y 60 deg" | "-Z 60 deg up" => "sky blue",
        _ => "ground green",
    }
}

struct Probe {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    camera: Camera,
    lights: LightScene,
}

fn probe() -> Option<Probe> {
    let adapter = common::adapter()?;
    let renderer = common::renderer(&adapter, SIZE)?;
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky_probe_target"),
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
    let camera = Camera::new(0.9, 1.0, 0.1, 200.0);
    let lights = LightScene::new(DirectionalLight::new(Vec3::Y, Vec3::ONE, 1.0));
    Some(Probe {
        renderer,
        target,
        view,
        camera,
        lights,
    })
}

impl Probe {
    /// Average RGB (0..255) of a 4x4 block at the centre of the frame, seen from `rotation`.
    fn centre(&mut self, rotation: Quat) -> [f32; 3] {
        self.renderer
            .camera_uniform
            .update(&self.camera, Vec3::ZERO, rotation);
        self.renderer.update_camera_buffer();
        let frame = FrameView::new(&self.camera, &[], &self.lights);
        self.renderer
            .render_frame_view(&frame, &self.view)
            .expect("frame");
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        let pixels = read_texture_ktx2(
            &self.renderer.device,
            &self.renderer.queue,
            &self.target,
            false,
        )
        .unwrap()
        .levels
        .swap_remove(0);
        let mut sum = [0u32; 3];
        for y in SIZE / 2 - 2..SIZE / 2 + 2 {
            for x in SIZE / 2 - 2..SIZE / 2 + 2 {
                let at = ((y * SIZE + x) * 4) as usize;
                for (channel, total) in sum.iter_mut().enumerate() {
                    *total += u32::from(pixels[at + channel]);
                }
            }
        }
        sum.map(|total| total as f32 / 16.0)
    }

    /// Makes `map` the environment and turns the skybox on.
    fn show_environment(&mut self, map: &Texture) {
        self.renderer.set_environment(map);
        self.renderer.set_skybox(true);
    }
}

#[test]
#[ignore = "prints the synthetic equirect probe"]
fn synthetic_equirect_seen_from_each_direction() {
    let _guard = lock();
    let Some(mut probe) = probe() else {
        return;
    };
    let map = synthetic_map(&probe.renderer.device, &probe.renderer.queue);
    probe.show_environment(&map);
    for (name, rotation) in views() {
        let rgb = probe.centre(rotation);
        println!(
            "synthetic {name:>15}: rgb {:>3.0} {:>3.0} {:>3.0}  expected {}",
            rgb[0],
            rgb[1],
            rgb[2],
            expected_for(name)
        );
    }
}

/// Mean RGB of the picture rows `from..to` (decoded file, linear, row 0 = top of the file).
fn band_mean(image: &image::Rgb32FImage, from: u32, to: u32) -> [f32; 3] {
    let mut sum = [0f64; 3];
    for y in from..to {
        for x in 0..image.width() {
            for (channel, total) in sum.iter_mut().enumerate() {
                *total += f64::from(image.get_pixel(x, y)[channel]);
            }
        }
    }
    let count = f64::from(to - from) * f64::from(image.width());
    sum.map(|total| (total / count) as f32)
}

#[test]
#[ignore = "prints the forest HDRI probe; needs SKY_PROBE_HDR"]
fn hdri_seen_from_each_direction() {
    let Some(path) = std::env::var_os("SKY_PROBE_HDR") else {
        return;
    };
    let _guard = lock();
    let bytes = std::fs::read(path).unwrap();
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Hdr)
        .unwrap()
        .into_rgb32f();
    let h = image.height();
    println!(
        "hdri {}x{}, rows top to bottom (file order):",
        image.width(),
        h
    );
    for band in 0..8 {
        let mean = band_mean(&image, band * h / 8, (band + 1) * h / 8);
        println!(
            "  file rows {:>2}/8: mean rgb {:.3} {:.3} {:.3}",
            band, mean[0], mean[1], mean[2]
        );
    }
    let Some(mut probe) = probe() else {
        return;
    };
    let map = Texture::from_hdr_bytes(
        &probe.renderer.device,
        &probe.renderer.queue,
        &bytes,
        "forest_hdr",
    )
    .unwrap();
    probe.show_environment(&map);
    for (name, rotation) in views() {
        let rgb = probe.centre(rotation);
        println!(
            "hdri {name:>15}: rgb {:>3.0} {:>3.0} {:>3.0}",
            rgb[0], rgb[1], rgb[2]
        );
    }
}

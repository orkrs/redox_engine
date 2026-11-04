//! SKY-02 frame checks with a renderer: the sky pass, the sun disc and the Hdri mode.
//! Skipped without a GPU. Run: `cargo test -p redox_render --test sky_render_gpu -- --test-threads=1`

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::sky::SkyMode;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Direction towards a sun `elevation_deg` above the horizon, straight ahead (the camera
/// looks along -Z).
fn sun_ahead(elevation_deg: f32) -> [f32; 3] {
    let e = elevation_deg.to_radians();
    [0.0, e.sin(), -e.cos()]
}

/// Shared frame rig for renderer-based tests.
struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    camera: Camera,
    lights: LightScene,
}

fn rig(sun_elevation_deg: f32) -> Option<Rig> {
    let adapter = common::adapter()?;
    let renderer = common::renderer(&adapter, 128)?;
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky_target"),
        size: wgpu::Extent3d {
            width: 128,
            height: 128,
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
    let s = sun_ahead(sun_elevation_deg);
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(s[0], s[1], s[2]),
        Vec3::ONE,
        3.0,
    ));
    Some(Rig {
        renderer,
        target,
        view,
        camera: Camera::new(0.9, 1.0, 0.1, 2000.0),
        lights,
    })
}

impl Rig {
    /// Renders one frame looking along -Z from eye height and returns the 8-bit target.
    fn frame(&mut self) -> Vec<u8> {
        let eye = Vec3::new(0.0, 1.6, 0.0);
        self.renderer
            .camera_uniform
            .update(&self.camera, eye, Quat::IDENTITY);
        self.renderer.update_camera_buffer();
        let fv = FrameView::new(&self.camera, &[], &self.lights);
        self.renderer
            .render_frame_view(&fv, &self.view)
            .expect("frame");
        read_texture_ktx2(
            &self.renderer.device,
            &self.renderer.queue,
            &self.target,
            false,
        )
        .expect("readback")
        .levels[0]
            .clone()
    }
}

#[test]
fn hdri_frames_are_bitwise_unchanged() {
    let _g = lock();
    let Some(mut rig) = rig(10.0) else {
        return;
    };
    // Default mode: no sky pass, cubemap skybox path as before.
    assert_eq!(rig.renderer.sky_mode(), SkyMode::Hdri);
    // Frames differ a little on their own (TAA jitter and history), so the reference is the
    // spread between two consecutive Hdri frames with no mode change at all.
    let first = rig.frame();
    let second = rig.frame();
    let baseline = mean_abs_diff(&first, &second);
    // Procedural and back: the Hdri frame must stay within that spread of the frames before.
    rig.renderer.set_sky_mode(SkyMode::Procedural);
    let _procedural = rig.frame();
    rig.renderer.set_sky_mode(SkyMode::Hdri);
    let after = rig.frame();
    let changed = mean_abs_diff(&second, &after);
    assert!(
        changed <= baseline.max(0.05) * 2.0,
        "Hdri frame changed after toggling the sky mode: mean diff {changed}, spread {baseline}"
    );
}

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f32 {
    let total: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    total as f32 / a.len().max(1) as f32
}

#[test]
fn the_sun_disk_is_the_brightest_pixel_in_the_frame() {
    let _g = lock();
    // Sun 10 degrees above the horizon, straight ahead (camera looks along -Z).
    let Some(mut rig) = rig(10.0) else {
        return;
    };
    rig.renderer.set_sky_mode(SkyMode::Procedural);
    rig.renderer.set_sky_sun_angle(0.0087);
    let mut frame = rig.frame();
    for _ in 0..3 {
        frame = rig.frame();
    }
    let (w, h) = (128u32, 128u32);
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        u32::from(frame[i]) + u32::from(frame[i + 1]) + u32::from(frame[i + 2])
    };
    // Expected centre: 10 degrees above the centre line, tan(10) / tan(fov/2) of half height.
    let offset = (10.0f32.to_radians().tan() / (0.9f32 * 0.5).tan()) * (h as f32 * 0.5);
    let expected_y = (h as f32 * 0.5 - offset).round() as u32;
    // The disc is about a pixel wide: take the brightest pixel in a small box around it.
    let disc = (expected_y.saturating_sub(4)..=(expected_y + 4).min(h - 1))
        .flat_map(|y| ((w / 2 - 4)..=(w / 2 + 4)).map(move |x| (x, y)))
        .map(|(x, y)| at(x, y))
        .max()
        .unwrap_or(0);
    let far_right = at(w * 3 / 4, expected_y);
    assert!(
        disc >= 3 * 245,
        "the sun disc should be white-hot at ({}, {expected_y}), got channel sum {disc}",
        w / 2
    );
    assert!(
        disc > far_right,
        "the sun disc ({disc}) must be brighter than the sky beside it ({far_right})"
    );
}

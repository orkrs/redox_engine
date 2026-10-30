//! Motion blur spreading beyond object edges (PST-03) on the real renderer. Skipped without a GPU.
//!
//! Spreading runs in the TAA branch like the other post effects. With the flag off (the default)
//! the frame must be byte-identical to a renderer without the tile passes; `post_fx.rs` covers
//! that path, so these tests only look at the flag being on.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::post::post_fx::MotionBlurSettings;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;

struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    mesh: usize,
    material: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(true);
    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(0.9, 0.9, 0.9)));
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("motion_blur_spread_target"),
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
    let camera = Camera::new(0.9, 1.0, 0.1, 100.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::ZERO, Quat::IDENTITY);
    renderer.update_camera_buffer();
    let mut lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, -0.3, -1.0),
        Vec3::ONE,
        3.0,
    ));
    lights.ambient = Vec3::splat(0.6);
    Some(Rig {
        renderer,
        target,
        view,
        mesh,
        material,
        camera,
        lights,
    })
}

impl Rig {
    fn object(&self, center: Vec3, scale: Vec3) -> RenderObject {
        RenderObject {
            id: 0,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
            color: [1.0; 4],
            mesh_index: self.mesh,
            material_index: self.material,
            bounds: Aabb::from_center_size(center, scale),
        }
    }

    /// Renders one frame and returns it as RGBA bytes.
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
}

fn luminance(frame: &[u8], x: u32, y: u32) -> f32 {
    let at = ((y * SIZE + x) * 4) as usize;
    (f32::from(frame[at]) + f32::from(frame[at + 1]) + f32::from(frame[at + 2])) / 3.0
}

/// Number of pixels in the middle row whose luminance differs from the top-left corner (the sky)
/// by more than `threshold`.
fn covered_width(frame: &[u8], threshold: f32) -> usize {
    let sky = luminance(frame, 1, 1);
    (0..SIZE)
        .filter(|&x| (luminance(frame, x, SIZE / 2) - sky).abs() > threshold)
        .count()
}

/// A cube flying along +x, 0.8 units per frame (about 20 pixels), 5 frames, at 5 units from the
/// camera; returns the last frame.
fn flying_cube(rig: &mut Rig) -> Vec<u8> {
    let mut last = Vec::new();
    for i in 0..5 {
        let x = -1.6 + 0.8 * i as f32;
        let cube = rig.object(Vec3::new(x, 0.0, -5.0), Vec3::ONE);
        last = rig.frame(&[cube]);
    }
    last
}

#[test]
fn spreading_is_off_by_default() {
    assert!(!MotionBlurSettings::default().spread_beyond_edges);
}

/// Wall time per frame in milliseconds over 30 flying-cube sequences (includes the readback).
fn millis_per_frame(rig: &mut Rig) -> f64 {
    let sequences = 30;
    let start = std::time::Instant::now();
    for _ in 0..sequences {
        flying_cube(rig);
    }
    start.elapsed().as_secs_f64() * 1000.0 / f64::from(sequences * 5)
}

/// Cost of the two tile passes and the spreading blur. Run with `--ignored --nocapture`; the
/// numbers go to `docs/BENCHMARKS.md`.
#[test]
#[ignore]
fn spreading_cost_per_frame() {
    let Some(mut rig) = rig() else { return };
    rig.renderer
        .set_motion_blur(Some(MotionBlurSettings::default()));
    let off = millis_per_frame(&mut rig);
    rig.renderer.set_motion_blur(Some(MotionBlurSettings {
        spread_beyond_edges: true,
        ..MotionBlurSettings::default()
    }));
    let on = millis_per_frame(&mut rig);
    println!("motion blur per frame: spreading off {off:.3} ms, on {on:.3} ms");
}

#[test]
fn the_edge_outside_a_fast_object_is_blurred_further_with_spreading_on() {
    let Some(mut without) = rig() else { return };
    without
        .renderer
        .set_motion_blur(Some(MotionBlurSettings::default()));
    let plain = flying_cube(&mut without);

    let Some(mut with) = rig() else { return };
    with.renderer.set_motion_blur(Some(MotionBlurSettings {
        spread_beyond_edges: true,
        ..MotionBlurSettings::default()
    }));
    let spread = flying_cube(&mut with);

    let plain_width = covered_width(&plain, 8.0);
    let spread_width = covered_width(&spread, 8.0);
    assert!(
        spread_width >= plain_width + 3,
        "spreading widened the blur by {} px, want at least 3 (plain {plain_width}, spread {spread_width})",
        spread_width as i64 - plain_width as i64
    );
}

//! Motion blur and depth of field (PST-01, PST-02) on the real renderer. Skipped without a GPU.
//!
//! Both effects live in the TAA branch of the post chain, so every test renders with TAA on. With an effect
//! off the frame must be byte-identical to a renderer that never heard of it.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::post::post_fx::{DofSettings, MotionBlurSettings};
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
        label: Some("post_fx_target"),
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

/// Number of pixels in the middle row whose luminance differs from the top-left corner (the sky) by more than `threshold`.
fn covered_width(frame: &[u8], threshold: f32) -> usize {
    let sky = luminance(frame, 1, 1);
    (0..SIZE)
        .filter(|&x| (luminance(frame, x, SIZE / 2) - sky).abs() > threshold)
        .count()
}

/// Largest luminance step between neighbouring pixels of the middle row inside `x_range`.
fn sharpest_edge(frame: &[u8], x_range: std::ops::Range<u32>) -> f32 {
    x_range
        .map(|x| (luminance(frame, x + 1, SIZE / 2) - luminance(frame, x, SIZE / 2)).abs())
        .fold(0.0, f32::max)
}

fn max_difference(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// A cube flying along +x, 0.8 units per frame (about 20 pixels), 5 frames, at 5 units from the camera; returns the last frame.
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
fn with_motion_blur_off_the_frame_is_unchanged() {
    let (Some(mut untouched), Some(mut explicit)) = (rig(), rig()) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    explicit.renderer.set_motion_blur(None);
    let a = flying_cube(&mut untouched);
    let b = flying_cube(&mut explicit);
    assert_eq!(a, b, "an explicit None must not change a single byte");
}

/// Width in pixels of the rise from the sky to the object on the left edge of the object that is on the right half
/// of the middle row: from 20% to 90% of the way between the sky and the brightest pixel.
fn rise_width(frame: &[u8]) -> u32 {
    let sky = luminance(frame, 1, 1);
    let top = (0..SIZE)
        .map(|x| luminance(frame, x, SIZE / 2))
        .fold(sky, f32::max);
    let start = (SIZE / 2..SIZE)
        .find(|&x| luminance(frame, x, SIZE / 2) > sky + 0.2 * (top - sky))
        .expect("an object edge");
    let end = (start..SIZE)
        .find(|&x| luminance(frame, x, SIZE / 2) >= sky + 0.9 * (top - sky))
        .expect("a plateau");
    end - start
}

/// The blur reads the velocity of the pixel it shades, so the moving object's own pixels are averaged along its
/// motion: the rise at its edge becomes a ramp several pixels wide. The still background next to it is left alone
/// (the outline does not grow), which is why the test measures the ramp and not the covered width.
#[test]
fn a_moving_object_gets_soft_edges_along_its_motion() {
    let (Some(mut sharp), Some(mut blurred)) = (rig(), rig()) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    blurred
        .renderer
        .set_motion_blur(Some(MotionBlurSettings::default()));
    let off = rise_width(&flying_cube(&mut sharp));
    let on = rise_width(&flying_cube(&mut blurred));
    assert!(
        on >= off + 2,
        "the edge should turn into a ramp: {off} -> {on} pixels"
    );
}

#[test]
fn a_still_scene_is_not_changed_by_motion_blur() {
    let (Some(mut sharp), Some(mut blurred)) = (rig(), rig()) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    blurred
        .renderer
        .set_motion_blur(Some(MotionBlurSettings::default()));
    let mut a = Vec::new();
    let mut b = Vec::new();
    for _ in 0..6 {
        let cube = sharp.object(Vec3::new(0.0, 0.0, -5.0), Vec3::ONE);
        a = sharp.frame(&[cube]);
        let cube = blurred.object(Vec3::new(0.0, 0.0, -5.0), Vec3::ONE);
        b = blurred.frame(&[cube]);
    }
    let worst = max_difference(&a, &b);
    assert!(worst <= 2, "a still scene changed by up to {worst}/255");
}

#[test]
fn the_effects_survive_a_resize() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.renderer
        .set_motion_blur(Some(MotionBlurSettings::default()));
    rig.renderer
        .set_depth_of_field(Some(DofSettings::default()));
    let cube = rig.object(Vec3::new(0.0, 0.0, -5.0), Vec3::ONE);
    rig.frame(std::slice::from_ref(&cube));
    rig.renderer.resize(SIZE, SIZE);
    let after = rig.frame(&[cube]);
    assert!(
        covered_width(&after, 25.0) > 5,
        "the frame after a resize must still show the cube"
    );
}

/// Two bright bars, one in focus (10 m) and one far behind it (40 m, made wider to cover the same screen width).
fn two_bars(rig: &mut Rig) -> Vec<u8> {
    let mut last = Vec::new();
    for _ in 0..6 {
        let near = rig.object(Vec3::new(-3.0, 0.0, -10.0), Vec3::new(1.5, 8.0, 1.0));
        let far = rig.object(Vec3::new(8.0, 0.0, -40.0), Vec3::new(6.0, 30.0, 1.0));
        last = rig.frame(&[near, far]);
    }
    last
}

#[test]
fn with_dof_off_the_frame_is_unchanged() {
    let (Some(mut untouched), Some(mut explicit)) = (rig(), rig()) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    explicit.renderer.set_depth_of_field(None);
    assert_eq!(two_bars(&mut untouched), two_bars(&mut explicit));
}

#[test]
fn a_far_object_loses_its_edge_sharpness_while_the_focused_one_keeps_it() {
    let (Some(mut sharp), Some(mut soft)) = (rig(), rig()) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    soft.renderer.set_depth_of_field(Some(DofSettings {
        focus_distance: 10.0,
        ..DofSettings::default()
    }));
    let off = two_bars(&mut sharp);
    let on = two_bars(&mut soft);
    // The near bar is on the left half of the image, the far one on the right half.
    let near_off = sharpest_edge(&off, 0..SIZE / 2 - 1);
    let near_on = sharpest_edge(&on, 0..SIZE / 2 - 1);
    let far_off = sharpest_edge(&off, SIZE / 2..SIZE - 1);
    let far_on = sharpest_edge(&on, SIZE / 2..SIZE - 1);
    assert!(near_off > 20.0 && far_off > 20.0, "bars must be visible");
    assert!(
        near_on >= near_off * 0.9,
        "the focused bar must stay sharp: {near_off} -> {near_on}"
    );
    assert!(
        far_on <= far_off * 0.6,
        "the far bar must soften: {far_off} -> {far_on}"
    );
}

#[test]
fn dof_and_motion_blur_can_run_together() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.renderer
        .set_motion_blur(Some(MotionBlurSettings::default()));
    rig.renderer
        .set_depth_of_field(Some(DofSettings::default()));
    let frame = flying_cube(&mut rig);
    assert!(covered_width(&frame, 25.0) > 5, "not black, cube visible");
}

//! Motion vectors of moving objects (TAA input): the velocity texture must carry the object's own
//! motion, only where the object is the visible surface, and must follow objects by id when the
//! object list changes. Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
/// Largest sub-pixel TAA jitter in UV plus rounding.
const JITTER: f32 = 0.02;

struct Rig {
    renderer: Renderer,
    target: wgpu::TextureView,
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
    let material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
    let target = common::target(&renderer.device, SIZE);
    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 5.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    Some(Rig {
        renderer,
        target,
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
            model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
            color: [1.0; 4],
            mesh_index: self.mesh,
            material_index: self.material,
            bounds: Aabb::from_center_size(center, scale),
        }
    }

    fn draw(&mut self, objects: &[RenderObject], ids: Option<&[u64]>) {
        let mut view = FrameView::new(&self.camera, objects, &self.lights);
        if let Some(ids) = ids {
            view = view.with_object_ids(ids);
        }
        self.renderer
            .render_frame_view(&view, &self.target)
            .unwrap();
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
    }

    /// Velocity (u, v) written for a pixel of the last frame.
    fn velocity_at(&self, x: u32, y: u32) -> (f32, f32) {
        let texture = &self.renderer.taa_velocity_tex;
        let image =
            read_texture_ktx2(&self.renderer.device, &self.renderer.queue, texture, false).unwrap();
        let (w, _) = (self.renderer.internal_width, self.renderer.internal_height);
        let at = ((y * w + x) * 8) as usize;
        let bytes = &image.levels[0];
        let half = |i: usize| {
            f16_to_f32(u16::from_le_bytes([
                bytes[at + i * 2],
                bytes[at + i * 2 + 1],
            ]))
        };
        (half(0), half(1))
    }
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f32::from(bits & 0x03ff);
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        e => sign * (1.0 + fraction / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Pixel that shows the point (x, 0) of the cube's front face (z = 0.5) from the test camera.
fn pixel_of(x: f32) -> (u32, u32) {
    let half_width = (0.9f32 / 2.0).tan() * 4.5;
    let ndc = x / half_width;
    (((ndc * 0.5 + 0.5) * SIZE as f32) as u32, SIZE / 2)
}

#[test]
fn a_moving_cube_carries_its_own_motion() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let size = Vec3::ONE;
    let before = rig.object(Vec3::new(-0.25, 0.0, 0.0), size);
    rig.draw(&[before], None);
    let after = rig.object(Vec3::new(0.25, 0.0, 0.0), size);
    rig.draw(&[after], None);

    let (px, py) = pixel_of(0.25);
    let (vx, vy) = rig.velocity_at(px, py);
    // The cube moved 0.5 m to the right: its front face is 4.5 m away, so that is 0.5 / (2 * 2.174)
    // = 0.115 of the screen width.
    assert!((vx - 0.115).abs() < JITTER, "horizontal motion {vx}");
    assert!(vy.abs() < JITTER, "no vertical motion {vy}");

    // The sky far from the cube only has the jitter, no object motion.
    let (sx, sy) = rig.velocity_at(8, 8);
    assert!(
        sx.abs() < JITTER && sy.abs() < JITTER,
        "sky motion {sx},{sy}"
    );
}

#[test]
fn a_cube_that_stands_still_has_no_object_motion() {
    let Some(mut rig) = rig() else {
        return;
    };
    let cube = rig.object(Vec3::ZERO, Vec3::ONE);
    rig.draw(std::slice::from_ref(&cube), None);
    rig.draw(&[cube], None);
    let (px, py) = pixel_of(0.0);
    let (vx, vy) = rig.velocity_at(px, py);
    assert!(
        vx.abs() < JITTER && vy.abs() < JITTER,
        "static object: {vx},{vy}"
    );
}

#[test]
fn a_hidden_mover_does_not_leak_its_motion_through_a_wall() {
    let Some(mut rig) = rig() else {
        return;
    };
    // A big static wall close to the camera hides a cube that moves behind it.
    let wall = rig.object(Vec3::new(0.0, 0.0, 2.0), Vec3::new(6.0, 6.0, 0.2));
    let behind_before = rig.object(Vec3::new(-0.25, 0.0, -1.0), Vec3::ONE);
    let behind_after = rig.object(Vec3::new(0.25, 0.0, -1.0), Vec3::ONE);
    rig.draw(&[wall.clone(), behind_before], None);
    rig.draw(&[wall, behind_after], None);
    let (vx, vy) = rig.velocity_at(SIZE / 2, SIZE / 2);
    assert!(
        vx.abs() < JITTER && vy.abs() < JITTER,
        "the wall pixel is static: {vx},{vy}"
    );
}

#[test]
fn ids_keep_the_motion_right_when_the_object_order_changes() {
    let Some(mut rig) = rig() else {
        return;
    };
    let wall = rig.object(Vec3::new(0.0, -3.0, 0.0), Vec3::new(8.0, 0.2, 8.0));
    let cube_before = rig.object(Vec3::new(-0.25, 0.0, 0.0), Vec3::ONE);
    let cube_after = rig.object(Vec3::new(0.25, 0.0, 0.0), Vec3::ONE);
    rig.draw(&[wall.clone(), cube_before], Some(&[10, 20]));
    // The list is reordered: without ids the cube would be compared with the wall.
    rig.draw(&[cube_after, wall], Some(&[20, 10]));
    let (px, py) = pixel_of(0.25);
    let (vx, _) = rig.velocity_at(px, py);
    assert!((vx - 0.115).abs() < JITTER, "motion follows the id: {vx}");
}

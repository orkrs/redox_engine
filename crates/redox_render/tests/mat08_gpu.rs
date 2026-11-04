//! GPU checks of MAT-08 on the real renderer: double-sided materials and the frames of
//! single-sided materials, which must stay bit-identical. Kept in its own binary: every test
//! makes a renderer, and the process-global pipeline cache keeps each device alive.
//! Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_quad, create_sphere};
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;

/// FNV-1a of the frame bytes of [`single_sided_frames_are_bitwise_unchanged`], recorded on the
/// renderer before MAT-08 step 2 changed any pipeline.
const SINGLE_SIDED_FRAME_HASH: u64 = 6546600085312106948;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Tests create renderers; running them at once can stall the driver, so they take turns.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    cube: usize,
    sphere: usize,
    quad: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    let cube = renderer.upload_mesh(&create_cube());
    let sphere = renderer.upload_mesh(&create_sphere(0.5, 32));
    let quad = renderer.upload_mesh(&create_quad());
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mat08_target"),
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
        sphere,
        quad,
        camera,
        lights,
    })
}

impl Rig {
    fn object(
        &self,
        mesh: usize,
        material: usize,
        id: u32,
        center: Vec3,
        scale: Vec3,
    ) -> RenderObject {
        RenderObject {
            id,
            shadow_class: ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, center),
            color: [1.0; 4],
            mesh_index: mesh,
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
}

/// 64-bit FNV-1a: a stable fingerprint of a frame.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// RGBA of the pixel at (`x`, `y`) of a `SIZE`-wide RGBA8 frame.
fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
}

/// Largest per-channel difference of two RGBA pixels.
fn pixel_diff(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// A quad 3 units in front of the camera, facing it with its front face when `turned` is
/// false and with its back face when `turned` is true (rotated half a turn about Y).
fn quad_object(rig: &Rig, material: usize, turned: bool) -> RenderObject {
    let mut quad = rig.object(
        rig.quad,
        material,
        1,
        Vec3::new(0.0, 0.0, -3.0),
        Vec3::splat(2.0),
    );
    let rotation = if turned {
        Quat::from_rotation_y(std::f32::consts::PI)
    } else {
        Quat::IDENTITY
    };
    quad.model_matrix = Mat4::from_scale_rotation_translation(
        Vec3::splat(2.0),
        rotation,
        Vec3::new(0.0, 0.0, -3.0),
    );
    quad
}

/// Light from the camera side, so the front face of the quad is fully lit.
fn light_from_camera(rig: &mut Rig) {
    let mut lights = LightScene::new(DirectionalLight::new(Vec3::Z, Vec3::ONE, 3.0));
    lights.ambient = Vec3::splat(0.4);
    rig.lights = lights;
}

/// The same brightness from both faces: the back of a double-sided quad is shaded like its
/// front (difference at most 3/255 per channel at the centre of the quad).
#[test]
fn a_double_sided_quad_is_lit_the_same_from_both_sides() {
    let _guard = lock();
    let Some(mut rig) = rig() else { return };
    light_from_camera(&mut rig);
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.7, 0.6, 0.5)).double_sided(true));
    let front = rig.frame(&[quad_object(&rig, material, false)]);
    let back = rig.frame(&[quad_object(&rig, material, true)]);
    let centre = SIZE / 2;
    let diff = pixel_diff(pixel(&front, centre, centre), pixel(&back, centre, centre));
    assert!(diff <= 3, "front and back differ by {diff}/255");
}

/// A single-sided quad seen from behind draws nothing: the frame equals an empty scene.
#[test]
fn a_single_sided_quad_is_invisible_from_behind() {
    let _guard = lock();
    let Some(mut rig) = rig() else { return };
    light_from_camera(&mut rig);
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.7, 0.6, 0.5)));
    let empty = rig.frame(&[]);
    let back = rig.frame(&[quad_object(&rig, material, true)]);
    assert!(
        empty == back,
        "single-sided back face changed the frame (max diff {})",
        pixel_diff(
            pixel(&empty, SIZE / 2, SIZE / 2),
            pixel(&back, SIZE / 2, SIZE / 2)
        )
    );
}

/// Single-sided materials on three shapes; the frame must match the recorded fingerprint.
#[test]
fn single_sided_frames_are_bitwise_unchanged() {
    let _guard = lock();
    let Some(mut rig) = rig() else { return };
    let red = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.8, 0.3, 0.2)));
    let grey = rig.renderer.add_material(Material::solid(Vec3::splat(0.6)));
    let objects = [
        rig.object(
            rig.quad,
            red,
            1,
            Vec3::new(-1.2, 0.0, -4.0),
            Vec3::splat(1.5),
        ),
        rig.object(rig.cube, grey, 2, Vec3::new(1.2, 0.0, -4.0), Vec3::ONE),
        rig.object(rig.sphere, red, 3, Vec3::new(0.0, -1.5, -5.0), Vec3::ONE),
    ];
    let mut frame = Vec::new();
    for _ in 0..4 {
        frame = rig.frame(&objects);
    }
    let hash = fnv1a(&frame);
    eprintln!("SINGLE_SIDED_FRAME_HASH = {hash}");
    assert_eq!(hash, SINGLE_SIDED_FRAME_HASH);
}

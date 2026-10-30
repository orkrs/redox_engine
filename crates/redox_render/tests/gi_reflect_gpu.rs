//! GPU checks of the rough reflections from the GI cache (GI-04). The pass runs in the TAA branch
//! of the post chain, so these tests enable TAA. Skipped without a GPU.

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
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Tests create two renderers each; running them at once can stall the driver, so they take turns.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Rig {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    cube: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig(taa: bool) -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(taa);
    let cube = renderer.upload_mesh(&create_cube());
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wave7_target"),
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

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------- cache reflections (GI-04)

use redox_render::gi::sdf_ref::cornell_box;
use redox_render::post::gi_reflect::GiReflectSettings;

/// A glossy metal floor in the Cornell room, seen from the green wall towards the red one.
/// `reflections`: `None` leaves the effect off. Returns the settled frame.
fn room_frame(reflections: Option<GiReflectSettings>, open_sky: bool) -> Option<Vec<u8>> {
    let mut rig = rig(true)?;
    let mut boxes = cornell_box().boxes;
    if open_sky {
        // Floor only: every reflected ray leaves the volume.
        boxes.truncate(1);
    }
    rig.renderer.set_gi_static_geometry(&boxes);
    rig.renderer.set_gi_reflections(reflections);
    rig.lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::ONE,
        0.0,
    ));
    rig.lights.ambient = Vec3::ZERO;
    let mut objects = Vec::new();
    for (i, b) in boxes.iter().enumerate() {
        let mut material = Material::solid(b.albedo);
        if i == 0 {
            material = material.metallic(1.0).roughness(0.3);
        }
        let slot = rig.renderer.add_material(material);
        let centre = (b.min + b.max) * 0.5;
        objects.push(rig.object(rig.cube, slot, i as u32, centre, b.max - b.min));
    }
    let eye = Vec3::new(1.6, 1.2, 0.0);
    let rotation =
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(-0.45);
    rig.renderer
        .camera_uniform
        .update(&rig.camera, eye, rotation);
    rig.renderer.update_camera_buffer();
    // GI needs a while to fill the probes before the cache has light to reflect.
    let mut frame = Vec::new();
    for _ in 0..200 {
        frame = rig.frame(&objects);
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    Some(frame)
}

fn mean_rgb(frame: &[u8], rect: (u32, u32, u32, u32)) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut n = 0.0;
    for y in rect.1..rect.3 {
        for x in rect.0..rect.2 {
            let p = px(frame, x, y);
            for c in 0..3 {
                sum[c] += p[c];
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

#[test]
fn a_glossy_floor_in_front_of_a_red_wall_reflects_red() {
    let _gpu = lock();
    let (Some(on), Some(off)) = (
        room_frame(Some(GiReflectSettings::default()), false),
        room_frame(None, false),
    ) else {
        return;
    };
    // The lower middle of the picture is the floor between the camera and the red wall.
    let rect = (SIZE / 4, SIZE * 5 / 8, SIZE * 3 / 4, SIZE - 4);
    let (a, b) = (mean_rgb(&on, rect), mean_rgb(&off, rect));
    // Off, the floor is grey; with the cache reflection it carries the colour of the red wall.
    assert!(
        (b[0] - b[1]).abs() < 8.0,
        "the floor without reflections is not grey: {b:?}"
    );
    assert!(
        a[0] > 1.5 * a[1] && a[0] > 1.5 * a[2],
        "the floor did not pick up red: on {a:?} off {b:?}"
    );
}

#[test]
fn a_ray_into_the_open_sky_is_a_miss() {
    let _gpu = lock();
    let (Some(on), Some(off)) = (
        room_frame(Some(GiReflectSettings::default()), true),
        room_frame(None, true),
    ) else {
        return;
    };
    let diff = max_diff(&on, &off);
    assert!(diff <= 2, "missed rays changed the frame by {diff}");
}

fn px(frame: &[u8], x: u32, y: u32) -> [f32; 3] {
    let at = ((y * SIZE + x) * 4) as usize;
    [
        f32::from(frame[at]),
        f32::from(frame[at + 1]),
        f32::from(frame[at + 2]),
    ]
}

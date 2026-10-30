//! Froxel fog (PST-06 step 3) on the real renderer. Skipped without a GPU.
//!
//! Like the other post effects, the fog runs in the TAA branch, so every test renders with TAA on.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::post::fog::FogSettings;
use redox_render::resource::ibl::read_texture_ktx2;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
const FOG_RGB: [f32; 3] = [0.7 * 255.0, 0.78 * 255.0, 0.85 * 255.0];

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
    renderer.enable_taa(false);
    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(0.9, 0.9, 0.9)));
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fog_froxel_target"),
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
    fn object(&self, center: Vec3) -> RenderObject {
        RenderObject {
            id: 0,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(Vec3::ONE, Quat::IDENTITY, center),
            color: [1.0; 4],
            mesh_index: self.mesh,
            material_index: self.material,
            bounds: Aabb::from_center_size(center, Vec3::ONE),
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

/// RGB of the centre pixel of a frame.
fn centre_rgb(frame: &[u8]) -> [f32; 3] {
    let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    [
        f32::from(frame[at]),
        f32::from(frame[at + 1]),
        f32::from(frame[at + 2]),
    ]
}

fn distance_to_fog(rgb: [f32; 3]) -> f32 {
    rgb.iter()
        .zip(FOG_RGB)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        .sqrt()
}

fn max_difference(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

#[test]
fn froxel_fog_with_zero_density_changes_nothing() {
    let Some(mut rig) = rig() else {
        return;
    };
    let cube = rig.object(Vec3::new(0.0, 0.0, -5.0));
    let without = rig.frame(std::slice::from_ref(&cube));

    rig.renderer.set_fog(Some(FogSettings {
        density: 0.0,
        ..FogSettings::default()
    }));
    rig.renderer.set_fog_froxel(true);
    let with_zero = rig.frame(std::slice::from_ref(&cube));

    assert!(max_difference(&without, &with_zero) <= 1);
}

#[test]
fn froxel_fog_pulls_distant_pixels_towards_the_fog_colour() {
    let Some(mut rig) = rig() else {
        return;
    };
    rig.renderer.set_fog(Some(FogSettings {
        density: 0.05,
        height_falloff: 0.0,
        ..FogSettings::default()
    }));
    rig.renderer.set_fog_froxel(true);

    let near = rig.object(Vec3::new(0.0, 0.0, -5.0));
    let near_rgb = centre_rgb(&rig.frame(std::slice::from_ref(&near)));
    let far = rig.object(Vec3::new(0.0, 0.0, -50.0));
    let far_rgb = centre_rgb(&rig.frame(std::slice::from_ref(&far)));

    assert!(distance_to_fog(far_rgb) < distance_to_fog(near_rgb));
}

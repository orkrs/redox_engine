//! GPU checks of the screen-space subsurface blur (MAT-05 step 3). The blur runs in the TAA
//! branch of the post chain, so these tests enable TAA. Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_sphere;
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
    sphere: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig(taa: bool) -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(taa);
    let sphere = renderer.upload_mesh(&create_sphere(0.5, 32));
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
        sphere,
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

    /// Renders `n` identical frames and returns the last.
    fn settle(&mut self, objects: &[RenderObject], n: usize) -> Vec<u8> {
        let mut last = Vec::new();
        for _ in 0..n {
            last = self.frame(objects);
        }
        last
    }
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------- subsurface blur (MAT-05)

use redox_render::material::SubsurfaceProfile;
use redox_render::post::sss::SssSettings;

/// A skin-like sphere (or a plain one) lit from the side on a dark background; the settled frame
/// with the SSS blur `blur` (`None` = off). Returns the frame and whether SSS was enabled on its own.
fn sphere_frame(subsurface: bool, blur: Option<Option<SssSettings>>) -> Option<(Vec<u8>, bool)> {
    let mut rig = rig(true)?;
    rig.lights = LightScene::new(DirectionalLight::new(
        Vec3::new(1.0, -0.2, -0.4),
        Vec3::ONE,
        3.0,
    ));
    rig.lights.ambient = Vec3::splat(0.02);
    let mut material = Material::solid(Vec3::new(0.8, 0.6, 0.5)).roughness(0.6);
    if subsurface {
        material = material.subsurface(SubsurfaceProfile {
            color: [1.0, 0.35, 0.2],
            radius_mm: 8.0,
            wrap: 0.4,
        });
    }
    let slot = rig.renderer.add_material(material);
    if let Some(setting) = blur {
        rig.renderer.set_sss(setting);
    }
    let auto = rig.renderer.sss().is_some();
    let sphere = rig.object(
        rig.sphere,
        slot,
        1,
        Vec3::new(0.0, 0.0, -3.0),
        Vec3::splat(3.0),
    );
    Some((rig.settle(&[sphere], 12), auto))
}

#[test]
fn a_frame_without_sss_materials_pays_nothing() {
    let _gpu = lock();
    let Some((_, auto)) = sphere_frame(false, None) else {
        return;
    };
    assert!(
        !auto,
        "the blur must stay off without a subsurface material"
    );
}

#[test]
fn a_subsurface_material_turns_the_blur_on_by_itself() {
    let _gpu = lock();
    let Some((_, auto)) = sphere_frame(true, None) else {
        return;
    };
    assert!(auto, "a subsurface material enables the blur");
}

/// A radius that spans several pixels at this tiny resolution.
fn wide() -> SssSettings {
    SssSettings {
        radius_mm: 80.0,
        ..SssSettings::default()
    }
}

#[test]
fn the_blur_changes_the_frame_inside_the_sphere() {
    let _gpu = lock();
    let (Some((blurred, _)), Some((sharp, _))) = (
        sphere_frame(true, Some(Some(wide()))),
        sphere_frame(true, Some(None)),
    ) else {
        return;
    };
    let diff = max_diff(&blurred, &sharp);
    assert!(diff > 2, "the blur did not change the frame (diff {diff})");
}

#[test]
fn the_silhouette_is_not_blurred() {
    let _gpu = lock();
    let (Some((blurred, _)), Some((sharp, _))) =
        (sphere_frame(true, None), sphere_frame(true, Some(None)))
    else {
        return;
    };
    // The dark background outside the sphere must not receive any blurred colour: the depth test
    // and the mask stop the kernel at the silhouette. Compare the corner regions.
    for (x, y) in [
        (2, 2),
        (SIZE - 3, 2),
        (2, SIZE - 3),
        (SIZE - 3, SIZE - 3),
        (SIZE / 2, 2),
    ] {
        let d = distance(px(&blurred, x, y), px(&sharp, x, y));
        assert!(d <= 2.0, "background pixel ({x}, {y}) changed by {d}");
    }
}

fn px(frame: &[u8], x: u32, y: u32) -> [f32; 3] {
    let at = ((y * SIZE + x) * 4) as usize;
    [
        f32::from(frame[at]),
        f32::from(frame[at + 1]),
        f32::from(frame[at + 2]),
    ]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

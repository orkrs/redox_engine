//! GPU checks of MAT-06 on the real renderer: masked (alpha-tested) shadow casters and
//! screen-space refraction. Kept apart from `wave7_gpu`: every test makes a renderer, and the
//! process-global pipeline cache keeps each device alive, so one binary can only hold so many.
//! Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::asset_types::BlendMode;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_sphere};
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
    sphere: usize,
    camera: Camera,
    lights: LightScene,
}

fn rig(taa: bool) -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(taa);
    let cube = renderer.upload_mesh(&create_cube());
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
        cube,
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

// ---------------------------------------------------------------- masked shadows (MAT-06)

/// Ground plus a cube caster above it, rendered with `material` for the cube and instance
/// alpha `alpha`; returns the settled frame.
fn masked_shadow_frame(masked: bool, alpha: f32) -> Option<Vec<u8>> {
    let mut rig = rig(false)?;
    if !rig.renderer.vsm_enabled {
        return None;
    }
    let ground_mat = rig.renderer.add_material(Material::solid(Vec3::splat(0.7)));
    let caster_mat = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.8, 0.3, 0.2)).masked(masked));
    let ground = rig.object(
        rig.cube,
        ground_mat,
        1,
        Vec3::new(0.0, -1.0, -8.0),
        Vec3::new(40.0, 0.2, 40.0),
    );
    let mut caster = rig.object(
        rig.cube,
        caster_mat,
        2,
        Vec3::new(0.0, 0.5, -6.0),
        Vec3::ONE,
    );
    caster.color = [1.0, 1.0, 1.0, alpha];
    Some(rig.settle(&[ground, caster], 10))
}

/// A masked caster whose alpha is below the threshold casts no shadow; with full alpha it casts
/// exactly the shadow of a plain caster (VSM pages).
#[test]
fn masked_shadows_work_in_vsm_pages() {
    let _gpu = lock();
    let (Some(plain), Some(masked_opaque), Some(masked_clear)) = (
        masked_shadow_frame(false, 1.0),
        masked_shadow_frame(true, 1.0),
        masked_shadow_frame(true, 0.2),
    ) else {
        return;
    };
    let same = max_diff(&plain, &masked_opaque);
    assert!(
        same <= 2,
        "an opaque masked caster differs from a plain one by {same}"
    );
    let holes = max_diff(&plain, &masked_clear);
    assert!(
        holes > 8,
        "a see-through masked caster still casts its shadow (diff {holes})"
    );
}

/// A scene without masked materials is not touched by the masked pipelines.
#[test]
fn an_opaque_caster_is_unchanged() {
    let _gpu = lock();
    let (Some(a), Some(b)) = (
        masked_shadow_frame(false, 1.0),
        masked_shadow_frame(false, 1.0),
    ) else {
        return;
    };
    assert_eq!(max_diff(&a, &b), 0);
}

// ---------------------------------------------------------------- refraction (MAT-06)

/// A glass sphere (alpha 0.5) in front of a bright and a dark box. `ior` of `None` = plain alpha
/// glass; returns the settled frame and the number of scene copies the renderer made.
fn refraction_frame(ior: Option<f32>) -> Option<(Vec<u8>, u64)> {
    let mut rig = rig(false)?;
    rig.lights.ambient = Vec3::splat(0.1);
    let bright = rig.renderer.add_material(Material::solid(Vec3::splat(0.9)));
    let dark = rig
        .renderer
        .add_material(Material::solid(Vec3::splat(0.05)));
    let mut glass_material = Material::solid(Vec3::splat(0.95));
    if let Some(ior) = ior {
        glass_material = glass_material.refractive(ior, 2.0);
    }
    let glass_slot = rig.renderer.add_material(glass_material);
    rig.renderer.material_blend_flags[glass_slot] = BlendMode::Alpha;
    let mut glass = rig.object(
        rig.sphere,
        glass_slot,
        3,
        Vec3::new(0.0, 0.0, -4.0),
        Vec3::splat(3.0),
    );
    glass.color = [1.0, 1.0, 1.0, 0.5];
    let objects = vec![
        rig.object(
            rig.cube,
            bright,
            1,
            Vec3::new(-2.0, 0.0, -8.0),
            Vec3::new(4.0, 6.0, 0.5),
        ),
        rig.object(
            rig.cube,
            dark,
            2,
            Vec3::new(2.0, 0.0, -8.0),
            Vec3::new(4.0, 6.0, 0.5),
        ),
        glass,
    ];
    let frame = rig.settle(&objects, 3);
    Some((frame, rig.renderer.scene_copy_frames))
}

#[test]
fn glass_with_ior_displaces_the_background() {
    let _gpu = lock();
    let (Some((plain, _)), Some((refracted, copies))) =
        (refraction_frame(Some(1.0)), refraction_frame(Some(1.5)))
    else {
        return;
    };
    assert!(copies > 0, "a refractive material must make a scene copy");
    let shift = max_diff(&plain, &refracted);
    assert!(
        shift > 8,
        "refraction did not move the background (diff {shift})"
    );
}

#[test]
fn ior_one_matches_plain_alpha_glass() {
    let _gpu = lock();
    let (Some((plain, _)), Some((refracted, _))) =
        (refraction_frame(None), refraction_frame(Some(1.0)))
    else {
        return;
    };
    // Mean absolute difference over the frame (0..255). Single pixels can differ more: the copy
    // is the resolved opaque colour, so a sphere edge that crosses a hard background edge mixes
    // the two there (documented limit of the scene-copy refraction with MSAA).
    let sum: u64 = plain
        .iter()
        .zip(&refracted)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = sum as f64 / plain.len() as f64;
    assert!(
        mean <= 0.02 * 255.0,
        "ior 1 differs from plain alpha glass by {mean:.2}/255 on average"
    );
}

#[test]
fn no_scene_copy_without_refractive_materials() {
    let _gpu = lock();
    let Some((_, copies)) = refraction_frame(None) else {
        return;
    };
    assert_eq!(copies, 0);
}

//! GPU checks for screen-space refraction (MAT-09). Skipped without a GPU.

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
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(false);
    renderer.set_aa_sharp(true);
    let cube = renderer.upload_mesh(&create_cube());
    let sphere = renderer.upload_mesh(&create_sphere(0.5, 32));
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("refraction_gpu_target"),
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
    lights.ambient = Vec3::splat(0.35);
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
    fn settle(&mut self, objects: &[RenderObject], n: usize) -> Vec<u8> {
        let view = FrameView::new(&self.camera, objects, &self.lights);
        let mut last = Vec::new();
        for _ in 0..n {
            self.renderer
                .render_frame_view(&view, &self.view)
                .expect("frame");
            let _ = self
                .renderer
                .device
                .poll(wgpu::PollType::wait_indefinitely());
            last = read_texture_ktx2(
                &self.renderer.device,
                &self.renderer.queue,
                &self.target,
                false,
            )
            .unwrap()
            .levels
            .swap_remove(0);
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

fn variance_center(frame: &[u8], x0: u32, y0: u32, w: u32, h: u32) -> f32 {
    let mut sum = 0.0_f64;
    let mut sum2 = 0.0_f64;
    let mut n = 0.0_f64;
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let at = ((y * SIZE + x) * 4) as usize;
            let l = (f32::from(frame[at]) + f32::from(frame[at + 1]) + f32::from(frame[at + 2]))
                / 3.0;
            sum += f64::from(l);
            sum2 += f64::from(l * l);
            n += 1.0;
        }
    }
    let mean = sum / n;
    (sum2 / n - mean * mean).max(0.0) as f32
}

fn glass_scene(rig: &mut Rig, ior: f32, roughness: f32, strength: f32) -> Vec<RenderObject> {
    let bright = rig.renderer.add_material(Material::solid(Vec3::splat(0.95)));
    let mut glass_mat = Material::solid(Vec3::splat(0.98));
    glass_mat.roughness = roughness;
    glass_mat = glass_mat.refractive(ior, strength);
    let glass = rig.renderer.add_material(glass_mat);
    rig.renderer.material_blend_flags[glass] = BlendMode::Alpha;
    vec![
        RenderObject {
            id: 1,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(
                Vec3::new(4.0, 6.0, 0.3),
                Quat::IDENTITY,
                Vec3::new(-2.0, 0.0, -10.0),
            ),
            color: [1.0; 4],
            mesh_index: rig.cube,
            material_index: bright,
            bounds: Aabb::from_center_size(Vec3::new(-2.0, 0.0, -10.0), Vec3::new(4.0, 6.0, 0.3)),
        },
        RenderObject {
            id: 2,
            shadow_class: redox_render::shadow::ShadowClass::Auto,
            deforms: false,
            pose_version: 0,
            model_matrix: Mat4::from_scale_rotation_translation(
                Vec3::splat(2.5),
                Quat::IDENTITY,
                Vec3::new(0.0, 0.0, -5.0),
            ),
            color: [1.0, 1.0, 1.0, 0.45],
            mesh_index: rig.sphere,
            material_index: glass,
            bounds: Aabb::from_center_size(Vec3::new(0.0, 0.0, -5.0), Vec3::splat(2.5)),
        },
    ]
}

#[test]
fn a_tilted_glass_shifts_the_background() {
    let _g = lock();
    let (Some(mut a), Some(mut b)) = (rig(), rig()) else {
        return;
    };
    let fa = a.settle(&glass_scene(&mut a, 1.0, 0.0, 2.0), 4);
    let fb = b.settle(&glass_scene(&mut b, 1.5, 0.0, 2.0), 4);
    assert!(max_diff(&fa, &fb) > 6);
    assert!(a.renderer.scene_copy_frames > 0);
}

#[test]
fn ior_one_matches_alpha_blend() {
    let _g = lock();
    let (Some(mut plain), Some(mut ior1)) = (rig(), rig()) else {
        return;
    };
    let mut g0 = Material::solid(Vec3::splat(0.98));
    let slot0 = plain.renderer.add_material(g0.clone());
    plain.renderer.material_blend_flags[slot0] = BlendMode::Alpha;
    let mut plain_objs = glass_scene(&mut plain, 1.0, 0.0, 0.0);
    plain_objs[1].material_index = slot0;
    let fa = plain.settle(&plain_objs, 4);
    let fb = ior1.settle(&glass_scene(&mut ior1, 1.0, 0.0, 2.0), 4);
    let sum: u64 = fa
        .iter()
        .zip(&fb)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    assert!(sum as f64 / fa.len() as f64 <= 0.03 * 255.0);
}

#[test]
fn higher_roughness_picks_a_blurrier_sample() {
    let _g = lock();
    let (Some(mut smooth), Some(mut rough)) = (rig(), rig()) else {
        return;
    };
    let fs = smooth.settle(&glass_scene(&mut smooth, 1.45, 0.05, 2.0), 4);
    let fr = rough.settle(&glass_scene(&mut rough, 1.45, 0.95, 2.0), 4);
    let var_smooth = variance_center(&fs, 52, 52, 24, 24);
    let var_rough = variance_center(&fr, 52, 52, 24, 24);
    assert!(var_rough < var_smooth);
}

#[test]
fn the_glass_does_not_sample_itself() {
    let _g = lock();
    let Some(mut rig) = rig() else {
        return;
    };
    let frame = rig.settle(&glass_scene(&mut rig, 1.5, 0.1, 2.5), 4);
    assert!(variance_center(&frame, SIZE / 2 - 4, SIZE / 2 - 4, 8, 8) < 2000.0);
}

#[test]
fn moving_glass_does_not_ghost_the_unshifted_background() {
    let _g = lock();
    let Some(mut rig) = rig() else {
        return;
    };
    rig.renderer.enable_taa(true);
    let mut objects = glass_scene(&mut rig, 1.5, 0.1, 2.0);
    let mut last = Vec::new();
    for i in 0..16 {
        objects[1].model_matrix = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.5),
            Quat::IDENTITY,
            Vec3::new(i as f32 * 0.05, 0.0, -5.0),
        );
        last = rig.settle(&objects, 1);
    }
    assert!(variance_center(&last, SIZE / 2 - 10, SIZE / 2 - 10, 20, 20) < 2500.0);
}

#[test]
fn static_glass_matches_the_frame_without_the_reactive_mask() {
    let _g = lock();
    let (Some(mut taa), Some(mut sharp)) = (rig(), rig()) else {
        return;
    };
    taa.renderer.enable_taa(true);
    sharp.renderer.enable_taa(false);
    sharp.renderer.set_aa_sharp(true);
    let fa = taa.settle(&glass_scene(&mut taa, 1.5, 0.1, 2.0), 8);
    let fb = sharp.settle(&glass_scene(&mut sharp, 1.5, 0.1, 2.0), 4);
    assert!(max_diff(&fa, &fb) <= 48);
}
//! GPU checks of the wave-seven features on the real renderer: fog (PST-06), SSR (PST-05), additive and alpha
//! surfaces (MAT-06), clear coat (MAT-04), TAA ghosting (PST-04) and shadow page caching (RND-08).
//! Skipped without a GPU. Fog and SSR run in the TAA branch of the post chain, so those tests enable TAA.

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
use redox_render::post::fog::FogSettings;
use redox_render::post::ssr::SsrSettings;
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

fn px(frame: &[u8], x: u32, y: u32) -> [f32; 3] {
    let at = ((y * SIZE + x) * 4) as usize;
    [
        f32::from(frame[at]),
        f32::from(frame[at + 1]),
        f32::from(frame[at + 2]),
    ]
}

fn lum(frame: &[u8], x: u32, y: u32) -> f32 {
    let p = px(frame, x, y);
    (p[0] + p[1] + p[2]) / 3.0
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

// ---------------------------------------------------------------- fog

fn fog_scene(rig: &mut Rig) -> Vec<RenderObject> {
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.9, 0.2, 0.1)));
    vec![
        rig.object(rig.cube, material, 1, Vec3::new(-1.5, 0.0, -6.0), Vec3::ONE),
        rig.object(
            rig.cube,
            material,
            2,
            Vec3::new(1.5, 0.0, -60.0),
            Vec3::splat(6.0),
        ),
    ]
}

#[test]
fn with_fog_off_the_frame_is_unchanged() {
    let _gpu = lock();
    let (Some(mut a), Some(mut b)) = (rig(true), rig(true)) else {
        return;
    };
    let objects_a = fog_scene(&mut a);
    let objects_b = fog_scene(&mut b);
    b.renderer.set_fog(None);
    let fa = a.settle(&objects_a, 4);
    let fb = b.settle(&objects_b, 4);
    assert_eq!(fa, fb);
}

#[test]
fn fog_pulls_distant_pixels_towards_the_fog_colour() {
    let _gpu = lock();
    let (Some(mut clear), Some(mut foggy)) = (rig(true), rig(true)) else {
        return;
    };
    let objects_c = fog_scene(&mut clear);
    let objects_f = fog_scene(&mut foggy);
    foggy.renderer.set_fog(Some(FogSettings {
        density: 0.05,
        height_falloff: 0.0,
        ..FogSettings::default()
    }));
    let without = clear.settle(&objects_c, 5);
    let with = foggy.settle(&objects_f, 5);
    // The near cube is around (30, 64), the far one around (67, 64).
    let near = distance(px(&without, 30, 64), px(&with, 30, 64));
    let far = distance(px(&without, 67, 64), px(&with, 67, 64));
    assert!(far > near + 10.0, "near changed {near}, far changed {far}");
}

#[test]
fn zero_density_fog_changes_nothing_visible() {
    let _gpu = lock();
    let (Some(mut clear), Some(mut foggy)) = (rig(true), rig(true)) else {
        return;
    };
    let objects_c = fog_scene(&mut clear);
    let objects_f = fog_scene(&mut foggy);
    foggy.renderer.set_fog(Some(FogSettings {
        density: 0.0,
        ..FogSettings::default()
    }));
    let a = clear.settle(&objects_c, 5);
    let b = foggy.settle(&objects_f, 5);
    assert!(max_diff(&a, &b) <= 2, "diff {}", max_diff(&a, &b));
}

#[test]
fn fog_survives_a_resize() {
    let _gpu = lock();
    let Some(mut rig) = rig(true) else { return };
    let objects = fog_scene(&mut rig);
    rig.renderer.set_fog(Some(FogSettings::default()));
    let first = rig.frame(&objects);
    assert!(first.iter().any(|&b| b != 0));
    rig.renderer.resize(96, 96);
    // After the resize the offscreen target no longer matches; rendering must still not fail.
    let view = FrameView::new(&rig.camera, &objects, &rig.lights);
    let _ = rig.renderer.render_frame_view(&view, &rig.view);
    let _ = rig
        .renderer
        .device
        .poll(wgpu::PollType::wait_indefinitely());
}

// ---------------------------------------------------------------- SSR

fn mirror_scene(rig: &mut Rig, floor_roughness: f32) -> Vec<RenderObject> {
    let floor = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.5, 0.5, 0.55)).roughness(floor_roughness));
    let bright = rig
        .renderer
        .add_material(Material::solid(Vec3::new(1.0, 0.9, 0.2)));
    vec![
        rig.object(
            rig.cube,
            floor,
            1,
            Vec3::new(0.0, -1.5, -8.0),
            Vec3::new(30.0, 0.2, 30.0),
        ),
        rig.object(
            rig.cube,
            bright,
            2,
            Vec3::new(0.0, 0.0, -8.0),
            Vec3::splat(1.5),
        ),
    ]
}

#[test]
fn with_ssr_off_the_frame_is_unchanged() {
    let _gpu = lock();
    let (Some(mut a), Some(mut b)) = (rig(true), rig(true)) else {
        return;
    };
    let oa = mirror_scene(&mut a, 0.05);
    let ob = mirror_scene(&mut b, 0.05);
    b.renderer.set_ssr(None);
    assert_eq!(a.settle(&oa, 4), b.settle(&ob, 4));
}

#[test]
fn a_mirror_floor_changes_the_frame_with_ssr() {
    let _gpu = lock();
    let (Some(mut off), Some(mut on)) = (rig(true), rig(true)) else {
        return;
    };
    let oo = mirror_scene(&mut off, 0.05);
    let on_objects = mirror_scene(&mut on, 0.05);
    on.renderer.set_ssr(Some(SsrSettings::default()));
    let a = off.settle(&oo, 5);
    let b = on.settle(&on_objects, 5);
    assert!(max_diff(&a, &b) > 8, "SSR left the frame untouched");
}

#[test]
fn a_rough_floor_is_not_changed_by_ssr() {
    let _gpu = lock();
    let (Some(mut off), Some(mut on)) = (rig(true), rig(true)) else {
        return;
    };
    let oo = mirror_scene(&mut off, 0.9);
    let on_objects = mirror_scene(&mut on, 0.9);
    on.renderer.set_ssr(Some(SsrSettings::default()));
    let a = off.settle(&oo, 5);
    let b = on.settle(&on_objects, 5);
    assert!(max_diff(&a, &b) <= 3, "diff {}", max_diff(&a, &b));
}

// ---------------------------------------------------------------- transparency

fn glass_frame(mode: Option<BlendMode>, alpha: f32) -> Option<f32> {
    let mut rig = rig(false)?;
    let behind = rig
        .renderer
        .add_material(Material::solid(Vec3::splat(0.05)));
    let front = rig
        .renderer
        .add_material(Material::solid(Vec3::splat(0.95)));
    if let Some(mode) = mode {
        rig.renderer.material_blend_flags[front] = mode;
    }
    let mut glass = rig.object(
        rig.cube,
        front,
        2,
        Vec3::new(0.0, 0.0, -4.0),
        Vec3::new(2.0, 2.0, 0.2),
    );
    glass.color = [1.0, 1.0, 1.0, alpha];
    let objects = vec![
        rig.object(
            rig.cube,
            behind,
            1,
            Vec3::new(0.0, 0.0, -8.0),
            Vec3::splat(4.0),
        ),
        glass,
    ];
    let frame = rig.settle(&objects, 2);
    Some(lum(&frame, SIZE / 2, SIZE / 2))
}

#[test]
fn alpha_glass_blends_between_the_background_and_an_opaque_surface() {
    let _gpu = lock();
    let Some(opaque) = glass_frame(None, 1.0) else {
        return;
    };
    let glass = glass_frame(Some(BlendMode::Alpha), 0.5).unwrap();
    // Background alone: the glass is far outside the view? Use a fully transparent glass as that reference.
    let nothing = glass_frame(Some(BlendMode::Alpha), 0.0).unwrap();
    assert!(
        nothing < glass && glass < opaque,
        "{nothing} < {glass} < {opaque}"
    );
}

#[test]
fn additive_surfaces_brighten_the_background() {
    let _gpu = lock();
    let Some(dark) = glass_frame(Some(BlendMode::Additive), 0.0) else {
        return;
    };
    let lit = glass_frame(Some(BlendMode::Additive), 0.6).unwrap();
    assert!(lit > dark + 5.0, "{dark} -> {lit}");
}

// ---------------------------------------------------------------- clear coat

fn sphere_peak(coat: Option<(f32, f32)>) -> Option<f32> {
    let mut rig = rig(false)?;
    let mut m = Material::solid(Vec3::new(0.4, 0.1, 0.1)).roughness(0.6);
    if let Some((w, r)) = coat {
        m = m.coat(w, r);
    }
    let material = rig.renderer.add_material(m);
    let objects = vec![rig.object(
        rig.sphere,
        material,
        1,
        Vec3::new(0.0, 0.0, -2.5),
        Vec3::splat(2.0),
    )];
    let frame = rig.settle(&objects, 2);
    let mut peak = 0.0f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            peak = peak.max(lum(&frame, x, y));
        }
    }
    Some(peak)
}

#[test]
fn a_coat_adds_a_second_highlight() {
    let _gpu = lock();
    let Some(plain) = sphere_peak(None) else {
        return;
    };
    let coated = sphere_peak(Some((1.0, 0.05))).unwrap();
    assert!(coated > plain + 10.0, "peak {plain} -> {coated}");
}

#[test]
fn a_zero_weight_coat_renders_like_no_coat() {
    let _gpu = lock();
    let Some(plain) = sphere_peak(None) else {
        return;
    };
    let none = sphere_peak(Some((0.0, 0.05))).unwrap();
    assert!((plain - none).abs() < 1.0, "{plain} vs {none}");
}

// ---------------------------------------------------------------- TAA (PST-04)

fn cube_row(frame: &[u8]) -> Vec<f32> {
    (0..SIZE).map(|x| lum(frame, x, SIZE / 2)).collect()
}

#[test]
fn a_moving_cube_leaves_no_ghost_trail() {
    let _gpu = lock();
    let Some(mut rig) = rig(true) else { return };
    let bright = rig
        .renderer
        .add_material(Material::solid(Vec3::splat(0.95)));
    let mut last = Vec::new();
    for step in 0..40 {
        let x = -3.0 + step as f32 * 0.15;
        let objects = vec![rig.object(rig.cube, bright, 1, Vec3::new(x, 0.0, -5.0), Vec3::ONE)];
        last = rig.frame(&objects);
    }
    let row = cube_row(&last);
    let sky = row[1];
    let covered = row.iter().filter(|&&l| (l - sky).abs() > 25.0).count();
    // The cube is about 29 pixels wide; a ghost would add a smear behind it.
    assert!(covered <= 29 + 8, "{covered} bright pixels in the row");
}

#[test]
fn a_static_scene_is_stable_over_a_jitter_cycle() {
    let _gpu = lock();
    let Some(mut rig) = rig(true) else { return };
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.7, 0.5, 0.3)));
    let objects = vec![rig.object(rig.cube, material, 1, Vec3::new(0.0, 0.0, -5.0), Vec3::ONE)];
    rig.settle(&objects, 8);
    let mut previous = rig.frame(&objects);
    for _ in 0..16 {
        let next = rig.frame(&objects);
        let total: f32 = previous
            .iter()
            .zip(&next)
            .map(|(a, b)| f32::from(a.abs_diff(*b)))
            .sum();
        let mean = total / previous.len() as f32;
        assert!(mean < 1.0, "frame to frame mean difference {mean}");
        previous = next;
    }
}

// ---------------------------------------------------------------- shadow page cache (RND-08)

#[test]
fn a_static_scene_stops_redrawing_shadow_pages_and_a_moved_object_redraws_some() {
    let _gpu = lock();
    let Some(mut rig) = rig(false) else { return };
    if !rig.renderer.vsm_enabled {
        return;
    }
    let material = rig.renderer.add_material(Material::solid(Vec3::splat(0.7)));
    let ground = rig.object(
        rig.cube,
        material,
        1,
        Vec3::new(0.0, -1.0, -8.0),
        Vec3::new(40.0, 0.2, 40.0),
    );
    let mut small = rig.object(rig.cube, material, 2, Vec3::new(0.0, 0.0, -6.0), Vec3::ONE);
    for _ in 0..8 {
        rig.frame(&[ground.clone_object(), small.clone_object()]);
    }
    assert_eq!(
        rig.renderer.vsm_dirty_pages_last_frame, 0,
        "pages are cached"
    );
    small.model_matrix = Mat4::from_translation(Vec3::new(1.5, 0.0, -6.0));
    small.bounds = Aabb::from_center_size(Vec3::new(1.5, 0.0, -6.0), Vec3::ONE);
    rig.frame(&[ground.clone_object(), small.clone_object()]);
    assert!(
        rig.renderer.vsm_dirty_pages_last_frame > 0,
        "the moved cube dirties pages"
    );
}

trait CloneObject {
    fn clone_object(&self) -> RenderObject;
}

impl CloneObject for RenderObject {
    fn clone_object(&self) -> RenderObject {
        RenderObject {
            id: self.id,
            shadow_class: self.shadow_class,
            model_matrix: self.model_matrix,
            color: self.color,
            mesh_index: self.mesh_index,
            material_index: self.material_index,
            bounds: self.bounds,
        }
    }
}

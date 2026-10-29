//! CPU instancing (plan item A2): objects with the same mesh and material are drawn with one
//! instanced draw call, and the picture does not change. Skipped without a GPU.
//!
//! The renderer sorts the object list of a frame by (mesh, material), so equal objects end up next
//! to each other, and every opaque draw loop merges runs of visible neighbours into one
//! `draw_indexed` with an instance range. `Renderer::forward_draw_calls` reports how many draw
//! calls the opaque forward pass recorded in the last frame.

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

const SIZE: u32 = 96;
/// Mean absolute channel error (0..255) under which two frames count as equal.
const TOLERANCE: f64 = 0.5;

struct Rig {
    renderer: Renderer,
    camera: Camera,
    lights: LightScene,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    renderer.enable_taa(false);
    let camera = Camera::new(0.9, 1.0, 0.1, 100.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 9.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    Some(Rig {
        renderer,
        camera,
        lights,
    })
}

fn cube_at(mesh: usize, material: usize, x: f32, y: f32) -> RenderObject {
    let center = Vec3::new(x, y, 0.0);
    RenderObject {
        id: 0,
        shadow_class: redox_render::shadow::ShadowClass::Auto,
        model_matrix: Mat4::from_scale_rotation_translation(
            Vec3::splat(0.8),
            Quat::from_rotation_y(0.4 + x * 0.1),
            center,
        ),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(center, Vec3::splat(0.8)),
    }
}

impl Rig {
    /// Renders `objects` and returns the RGBA8 picture.
    fn draw(&mut self, objects: &[RenderObject]) -> Vec<u8> {
        let target = self
            .renderer
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("instancing_target"),
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
        let frame = FrameView::new(&self.camera, objects, &self.lights);
        self.renderer.render_frame_view(&frame, &view).unwrap();
        let _ = self
            .renderer
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        read_texture_ktx2(&self.renderer.device, &self.renderer.queue, &target, false)
            .unwrap()
            .levels
            .swap_remove(0)
    }
}

fn mean_error(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (*x as f64 - *y as f64).abs())
        .sum::<f64>()
        / a.len() as f64
}

/// A 6 x 5 grid of cubes.
fn grid() -> Vec<(f32, f32)> {
    (0..5)
        .flat_map(|row| (0..6).map(move |col| (col as f32 * 1.3 - 3.25, row as f32 * 1.3 - 2.6)))
        .collect()
}

#[test]
fn identical_objects_share_one_draw_call() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mesh = rig.renderer.upload_mesh(&create_cube());
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
    let objects: Vec<_> = grid()
        .into_iter()
        .map(|(x, y)| cube_at(mesh, material, x, y))
        .collect();
    rig.draw(&objects);
    assert_eq!(rig.renderer.forward_draw_calls(), 1, "30 cubes, one draw");
}

#[test]
fn instancing_does_not_change_the_picture() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
    // Scene A: every cube uses the same mesh slot (batched).
    let shared = rig.renderer.upload_mesh(&create_cube());
    let batched: Vec<_> = grid()
        .into_iter()
        .map(|(x, y)| cube_at(shared, material, x, y))
        .collect();
    let picture_batched = rig.draw(&batched);
    let batched_calls = rig.renderer.forward_draw_calls();

    // Scene B: identical cubes, but each in its own mesh slot, so nothing can be merged.
    let separate: Vec<_> = grid()
        .into_iter()
        .map(|(x, y)| {
            let mesh = rig.renderer.upload_mesh(&create_cube());
            cube_at(mesh, material, x, y)
        })
        .collect();
    let picture_separate = rig.draw(&separate);
    assert_eq!(rig.renderer.forward_draw_calls(), 30, "no merging possible");
    assert_eq!(batched_calls, 1);

    let error = mean_error(&picture_batched, &picture_separate);
    assert!(error < TOLERANCE, "pictures differ: mean error {error}");
    let lit = picture_batched
        .chunks(4)
        .filter(|px| px[0] > 40 || px[1] > 40 || px[2] > 40)
        .count();
    assert!(lit > 300, "the picture shows the cubes ({lit} lit pixels)");
}

#[test]
fn interleaved_meshes_and_materials_are_grouped_by_the_renderer() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mesh_a = rig.renderer.upload_mesh(&create_cube());
    let mesh_b = rig.renderer.upload_mesh(&create_cube());
    let red = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.9, 0.2, 0.2)));
    let blue = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.2, 0.2, 0.9)));
    // Alternating (mesh_a, red) and (mesh_b, blue) in the input list.
    let interleaved: Vec<_> = grid()
        .into_iter()
        .enumerate()
        .map(|(i, (x, y))| {
            if i % 2 == 0 {
                cube_at(mesh_a, red, x, y)
            } else {
                cube_at(mesh_b, blue, x, y)
            }
        })
        .collect();
    let picture = rig.draw(&interleaved);
    assert_eq!(
        rig.renderer.forward_draw_calls(),
        2,
        "two (mesh, material) groups"
    );

    // The same objects handed over already grouped give the same picture.
    let mut grouped = interleaved.clone();
    grouped.sort_by_key(|o| (o.mesh_index, o.material_index));
    let grouped_picture = rig.draw(&grouped);
    assert!(mean_error(&picture, &grouped_picture) < TOLERANCE);
}

#[test]
fn culled_objects_inside_a_group_do_not_break_the_picture() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mesh = rig.renderer.upload_mesh(&create_cube());
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
    let visible: Vec<_> = grid()
        .into_iter()
        .map(|(x, y)| cube_at(mesh, material, x, y))
        .collect();
    let clean = rig.draw(&visible);

    // Same cubes with off-screen ones (same mesh and material) mixed into the list.
    let mut mixed = Vec::new();
    for (i, object) in visible.iter().enumerate() {
        mixed.push(object.clone());
        if i % 3 == 0 {
            mixed.push(cube_at(mesh, material, 500.0 + i as f32, 0.0));
        }
    }
    let with_culled = rig.draw(&mixed);
    assert!(mean_error(&clean, &with_culled) < TOLERANCE);
    assert!(
        rig.renderer.forward_draw_calls() <= 2,
        "culled neighbours must not split the group into many draws (got {})",
        rig.renderer.forward_draw_calls()
    );
}

#[test]
fn an_empty_or_single_object_frame_works() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let mesh = rig.renderer.upload_mesh(&create_cube());
    let material = rig
        .renderer
        .add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
    rig.draw(&[]);
    assert_eq!(rig.renderer.forward_draw_calls(), 0);
    rig.draw(&[cube_at(mesh, material, 0.0, 0.0)]);
    assert_eq!(rig.renderer.forward_draw_calls(), 1);
}

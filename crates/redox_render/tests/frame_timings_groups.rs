//! Gate for the GPU-scene work (RND-07): CPU frame time as the number of (mesh, material) groups grows with the
//! object count fixed. Prints a table (run with `--release -- --nocapture`); only a sanity bound is asserted.
//! Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::{create_cube, create_quad, create_sphere, create_torus};
use redox_render::renderer::FrameCpuTimings;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;
const MESHES: usize = 5;
const MATERIALS: usize = 80;

struct Rig {
    renderer: Renderer,
    target: wgpu::TextureView,
    meshes: Vec<usize>,
    materials: Vec<usize>,
    camera: Camera,
    lights: LightScene,
}

fn rig() -> Option<Rig> {
    let adapter = common::adapter()?;
    let mut renderer = common::renderer(&adapter, SIZE)?;
    let meshes = vec![
        renderer.upload_mesh(&create_cube()),
        renderer.upload_mesh(&create_sphere(0.5, 16)),
        renderer.upload_mesh(&create_torus(0.4, 0.15, 16, 8)),
        renderer.upload_mesh(&create_quad()),
        renderer.upload_mesh(&create_sphere(0.5, 8)),
    ];
    assert_eq!(meshes.len(), MESHES);
    let materials = (0..MATERIALS)
        .map(|i| {
            let f = i as f32 / MATERIALS as f32;
            renderer.add_material(Material::solid(Vec3::new(f, 1.0 - f, 0.5)))
        })
        .collect();
    let target = common::target(&renderer.device, SIZE);
    let camera = Camera::new(0.9, 1.0, 0.1, 500.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 40.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    Some(Rig {
        renderer,
        target,
        meshes,
        materials,
        camera,
        lights,
    })
}

impl Rig {
    /// `count` objects spread over `groups` distinct (mesh, material) pairs, interleaved so that sorting has work to do.
    fn objects(&self, count: usize, groups: usize) -> Vec<RenderObject> {
        (0..count)
            .map(|i| {
                let group = (i * 7919) % groups; // a fixed shuffle
                let mesh = self.meshes[group % MESHES];
                let material = self.materials[(group / MESHES) % MATERIALS];
                let center = Vec3::new(
                    (i % 50) as f32 * 0.6 - 15.0,
                    (i / 50) as f32 * 0.6 - 8.0,
                    0.0,
                );
                RenderObject {
                    id: i as u32,
                    shadow_class: ShadowClass::Static,
                    model_matrix: Mat4::from_scale_rotation_translation(
                        Vec3::splat(0.4),
                        Quat::IDENTITY,
                        center,
                    ),
                    color: [1.0; 4],
                    mesh_index: mesh,
                    material_index: material,
                    bounds: Aabb::from_center_size(center, Vec3::splat(0.4)),
                }
            })
            .collect()
    }

    fn median(&mut self, count: usize, groups: usize, frames: usize) -> (FrameCpuTimings, u32) {
        let objects = self.objects(count, groups);
        let mut all = Vec::new();
        let mut calls = 0;
        for _ in 0..frames {
            let view = FrameView::new(&self.camera, &objects, &self.lights);
            self.renderer
                .render_frame_view(&view, &self.target)
                .unwrap();
            all.push(self.renderer.last_frame_cpu_timings());
            calls = self.renderer.forward_draw_calls();
            let _ = self
                .renderer
                .device
                .poll(wgpu::PollType::wait_indefinitely());
        }
        all.sort_by(|a, b| a.total_ms().total_cmp(&b.total_ms()));
        (all[all.len() / 2], calls)
    }
}

#[test]
fn cpu_time_grows_with_groups_not_objects() {
    let Some(mut rig) = rig() else { return };
    let mut totals = Vec::new();
    eprintln!("3000 objects, CPU frame time by number of (mesh, material) groups:");
    for groups in [1usize, 20, 100, 400] {
        let (t, calls) = rig.median(3000, groups, 9);
        eprintln!(
            "  groups {groups:>3}: draw calls {calls:>4}  prepare {:.2} ms  record {:.2} ms  submit {:.2} ms  total {:.2} ms",
            t.prepare_ms,
            t.record_ms,
            t.submit_ms,
            t.total_ms()
        );
        assert!(t.total_ms() < 1000.0);
        totals.push((groups, t.total_ms(), calls));
    }
    let (_, _, few) = totals[0];
    let (_, _, many) = totals[3];
    assert!(
        many > few,
        "more groups mean more draw calls: {few} vs {many}"
    );
}

//! CPU time of `render_frame_into` split into prepare / record / submit (RND-04). Skipped without a GPU.
//!
//! The numbers depend on the machine, so the tests only check that they are recorded, split sensibly and grow
//! with the scene. Run with `--release -- --nocapture` to print the figures for the plan (gate A1/A2).

#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::Renderer;
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::renderer::FrameCpuTimings;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;

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
    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
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
        mesh,
        material,
        camera,
        lights,
    })
}

impl Rig {
    fn objects(&self, count: usize) -> Vec<RenderObject> {
        (0..count)
            .map(|i| {
                let center = Vec3::new(
                    (i % 50) as f32 * 0.6 - 15.0,
                    (i / 50) as f32 * 0.6 - 8.0,
                    0.0,
                );
                RenderObject {
                    model_matrix: Mat4::from_scale_rotation_translation(
                        Vec3::splat(0.4),
                        Quat::IDENTITY,
                        center,
                    ),
                    color: [1.0; 4],
                    mesh_index: self.mesh,
                    material_index: self.material,
                    bounds: Aabb::from_center_size(center, Vec3::splat(0.4)),
                }
            })
            .collect()
    }

    /// Renders `frames` frames and returns the timings of the median one by total time.
    fn median_timings(&mut self, count: usize, frames: usize) -> FrameCpuTimings {
        let objects = self.objects(count);
        let mut all = Vec::new();
        for _ in 0..frames {
            let view = FrameView::new(&self.camera, &objects, &self.lights);
            self.renderer
                .render_frame_view(&view, &self.target)
                .unwrap();
            all.push(self.renderer.last_frame_cpu_timings());
            let _ = self
                .renderer
                .device
                .poll(wgpu::PollType::wait_indefinitely());
        }
        all.sort_by(|a, b| a.total_ms().total_cmp(&b.total_ms()));
        all[all.len() / 2]
    }
}

#[test]
fn timings_are_recorded_and_split_into_three_parts() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    assert_eq!(
        rig.renderer.last_frame_cpu_timings(),
        FrameCpuTimings::default(),
        "nothing rendered yet"
    );
    let t = rig.median_timings(30, 3);
    assert!(t.total_ms() > 0.0);
    assert!(t.prepare_ms >= 0.0 && t.record_ms > 0.0 && t.submit_ms >= 0.0);
    assert!(
        t.total_ms() < 1000.0,
        "a frame of 30 cubes is not a second: {t:?}"
    );
}

#[test]
fn more_objects_do_not_make_the_prepare_step_cheaper() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let small = rig.median_timings(30, 5);
    let large = rig.median_timings(3000, 5);
    eprintln!("CPU frame, 30 objects:   {small:?}");
    eprintln!("CPU frame, 3000 objects: {large:?}");
    assert!(
        large.prepare_ms >= small.prepare_ms,
        "prepare: {} ms for 30 objects, {} ms for 3000",
        small.prepare_ms,
        large.prepare_ms
    );
}

//! RND-09 step 2: the frame graph declares which textures each pass reads and writes; the passes
//! report the textures they hand to the GPU, and a debug build compares the two after every frame.
//! Skipped without a GPU, and only meaningful in debug builds (the check is compiled out in release).
//!
//! The test is ignored until the declarations match the passes: run it with
//! `cargo test -p redox_render --test frame_graph_runtime -- --ignored --nocapture` to print the
//! differences.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;

const SIZE: u32 = 128;

#[test]
#[ignore = "declarations differ from the passes (RND-09 step 2 finding); run with --ignored"]
fn every_declared_read_and_write_is_used_by_the_pass() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    renderer.enable_taa(true);
    let mesh = renderer.upload_mesh(&create_cube());
    let material = renderer.add_material(Material::solid(Vec3::new(0.8, 0.4, 0.2)));
    let target = common::target(&renderer.device, SIZE);
    let camera = Camera::new(0.9, 1.0, 0.1, 50.0);
    renderer
        .camera_uniform
        .update(&camera, Vec3::new(0.0, 0.0, 5.0), Quat::IDENTITY);
    renderer.update_camera_buffer();
    let lights = LightScene::new(DirectionalLight::new(
        Vec3::new(0.3, -0.5, -1.0),
        Vec3::ONE,
        2.0,
    ));
    let center = Vec3::ZERO;
    let objects = vec![RenderObject {
        id: 0,
        shadow_class: ShadowClass::Auto,
        deforms: false,
        pose_version: 0,
        model_matrix: Mat4::from_scale_rotation_translation(Vec3::ONE, Quat::IDENTITY, center),
        color: [1.0; 4],
        mesh_index: mesh,
        material_index: material,
        bounds: Aabb::from_center_size(center, Vec3::ONE),
    }];
    let view = FrameView::new(&camera, &objects, &lights);
    renderer.render_frame_view(&view, &target).unwrap();
    let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());

    let mismatches = renderer.frame_usage_mismatches().to_vec();
    for m in &mismatches {
        eprintln!("{m:?}");
    }
    assert!(
        mismatches.is_empty(),
        "{} declaration mismatches",
        mismatches.len()
    );
}

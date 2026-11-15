//! RND-09 step 2 + RND-12: frame graph usage vs passes and transient pool behavior.
//!
//! GPU tests skip without an adapter. Pool unit tests are CPU-only.

#![cfg(debug_assertions)]
#![allow(clippy::unwrap_used)]

mod common;

use redox_math::{Aabb, Mat4, Quat, Vec3};
use redox_render::camera::Camera;
use redox_render::frame::FrameView;
use redox_render::frame_graph::{self, FrameFeatures};
use redox_render::light::{DirectionalLight, LightScene};
use redox_render::material::Material;
use redox_render::mesh::primitive::create_cube;
use redox_render::post::fog::FogSettings;
use redox_render::render_graph::{RenderGraph, TextureDesc};
use redox_render::shadow::ShadowClass;
use redox_render::systems::RenderObject;
use redox_render::transient_pool::{PoolKey, TransientPool};

const SIZE: u32 = 128;

#[test]
fn every_declared_read_and_write_is_used_by_the_pass() {
    let Some(adapter) = common::adapter() else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let Some(mut renderer) = common::renderer(&adapter, SIZE) else {
        return;
    };
    renderer.enable_taa(true);
    renderer.vsm_enabled = true;
    renderer.post_fx_settings.fog = Some(FogSettings::default());
    renderer.rebuild_transient_pool();
    let mesh = renderer.upload_mesh(&create_cube());
    assert_ne!(mesh, usize::MAX);
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

#[test]
fn declared_access_matches_the_frame() {
    every_declared_read_and_write_is_used_by_the_pass();
}

#[test]
fn two_nonoverlapping_targets_share_a_slot() {
    let mut g = RenderGraph::new();
    let desc = TextureDesc {
        width: 64,
        height: 64,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        sample_count: 1,
        mip_level_count: 1,
    };
    let a = g.create_texture("a", desc);
    let b = g.create_texture("b", desc);
    let c = g.create_texture("c", desc);
    let out = g.import_texture("out", desc);
    g.add_pass("p1", |p| {
        p.write(a);
    });
    g.add_pass("p2", |p| {
        p.read(a).write(b);
    });
    g.add_pass("p3", |p| {
        p.read(b).write(c);
    });
    g.add_pass("p4", |p| {
        p.read(c).write(out);
    });
    let comp = g.compile().unwrap();
    assert_eq!(comp.slot_of(a), comp.slot_of(c));
    assert_eq!(comp.slot_count, 2);
}

#[test]
fn resize_keeps_an_unchanged_slot() {
    let Some(adapter) = common::adapter() else {
        return;
    };
    let Some(device) = common::device_with_bc(&adapter).map(|(d, _)| d) else {
        return;
    };
    let graph = frame_graph::frame_graph(64, 64).compile().unwrap();
    let key = PoolKey {
        width: 64,
        height: 64,
        features: FrameFeatures::default(),
    };
    let mut pool = TransientPool::new();
    pool.reset_creation_counter();
    pool.rebuild(&device, &graph, key);
    let created_first = pool.textures_created();
    pool.rebuild(&device, &graph, key);
    assert_eq!(
        pool.textures_created(),
        created_first,
        "unchanged size reuses slots"
    );
}

#[test]
fn disabling_fog_drops_its_transient_target() {
    let with = frame_graph::frame_graph_with_features(320, 180, FrameFeatures { fog: true })
        .compile()
        .unwrap();
    let without = frame_graph::frame_graph_with_features(320, 180, FrameFeatures { fog: false })
        .compile()
        .unwrap();
    let fog_transient = |c: &redox_render::render_graph::CompiledGraph| {
        c.logical_transients()
            .iter()
            .any(|(_, name)| *name == "fog_scratch")
    };
    assert!(fog_transient(&with));
    assert!(!fog_transient(&without));
    assert!(with.slot_count >= without.slot_count);
}

#[test]
fn a_partial_write_does_not_alias() {
    let mut g = RenderGraph::new();
    let desc = TextureDesc {
        width: 32,
        height: 32,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        sample_count: 1,
        mip_level_count: 1,
    };
    let a = g.create_texture("mask", desc);
    let b = g.create_texture("full", desc);
    let screen = g.import_texture("screen", desc);
    g.add_pass("partial", |p| {
        p.write_partial(a);
    });
    g.add_pass("fullpass", |p| {
        p.write(b);
    });
    g.add_pass("out", |p| {
        p.read(b).write(screen);
    });
    let c = g.compile().unwrap();
    assert_ne!(c.slot_of(a), c.slot_of(b));
}

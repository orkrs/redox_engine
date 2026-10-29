//! Morph targets on the GPU (ANM-03): the compute skinning must match the CPU path. Skipped without a GPU.

#![allow(clippy::unwrap_used)]

mod common;

use redox_anim::procedural::demo_rig;
use redox_anim::{MorphTarget, Pose};
use redox_math::Mat4;
use redox_render::mesh::compress::CompressedVertex;
use redox_render::{Renderer, SkinnedMeshBinding};

fn renderer() -> Option<Renderer> {
    let adapter = common::adapter()?;
    common::renderer(&adapter, 64)
}

fn vertices(r: &Renderer, slot: usize) -> Vec<CompressedVertex> {
    let bytes = r.read_mesh_vertex_bytes(slot).unwrap();
    bytemuck::cast_slice(&bytes).to_vec()
}

fn rest_pose(model: &redox_anim::SkinnedModel) -> Vec<Mat4> {
    let local = model.clips[0].sample(&model.skeleton, 0.0, true);
    Pose::from_local(&model.skeleton, &local).skin_matrices(&model.skeleton)
}

/// The demo rig with two morph targets: one lifts every vertex, one pushes vertices sideways.
fn morphed_rig() -> redox_anim::SkinnedModel {
    let mut model = demo_rig();
    let n = model.geometry.vertex_count();
    model.geometry.morph_targets = vec![
        MorphTarget {
            name: "lift".into(),
            positions: vec![[0.0, 0.1, 0.0]; n],
            ..Default::default()
        },
        MorphTarget {
            name: "push".into(),
            positions: vec![[0.08, 0.0, 0.0]; n],
            ..Default::default()
        },
    ];
    model
}

fn max_diff(a: [i16; 4], b: [i16; 4]) -> i32 {
    (0..3)
        .map(|k| (i32::from(a[k]) - i32::from(b[k])).abs())
        .max()
        .unwrap_or(0)
}

#[test]
fn gpu_morphs_match_the_cpu_reference() {
    let Some(mut r) = renderer() else { return };
    let model = morphed_rig();
    let skin = rest_pose(&model);
    let mut cpu = SkinnedMeshBinding::new(&mut r, &model.geometry, 2.0);
    let mut gpu = SkinnedMeshBinding::new(&mut r, &model.geometry, 2.0);
    for weights in [[0.0f32, 0.0], [1.0, 0.0], [0.5, 0.25], [0.0, 1.0]] {
        cpu.set_morph_weights(&r, &weights);
        gpu.set_morph_weights(&r, &weights);
        assert!(cpu.update(&r, &skin));
        assert!(gpu.update_gpu(&r, &skin));
        let expected = vertices(&r, cpu.mesh_slot);
        let actual = vertices(&r, gpu.mesh_slot);
        for (v, (e, a)) in expected.iter().zip(&actual).enumerate() {
            assert!(
                max_diff(e.position, a.position) <= 4,
                "weights {weights:?} vertex {v}: {:?} vs {:?}",
                e.position,
                a.position
            );
        }
    }
}

#[test]
fn updating_the_weights_changes_the_next_dispatch() {
    let Some(mut r) = renderer() else { return };
    let model = morphed_rig();
    let skin = rest_pose(&model);
    let mut gpu = SkinnedMeshBinding::new(&mut r, &model.geometry, 2.0);
    gpu.set_morph_weights(&r, &[0.0, 0.0]);
    assert!(gpu.update_gpu(&r, &skin));
    let before = vertices(&r, gpu.mesh_slot);
    gpu.set_morph_weights(&r, &[1.0, 0.0]);
    assert!(gpu.update_gpu(&r, &skin));
    let after = vertices(&r, gpu.mesh_slot);
    assert_ne!(
        before.iter().map(|v| v.position).collect::<Vec<_>>(),
        after.iter().map(|v| v.position).collect::<Vec<_>>()
    );
}

#[test]
fn zero_weights_equal_the_unmorphed_output() {
    let Some(mut r) = renderer() else { return };
    let plain = demo_rig();
    let morphed = morphed_rig();
    let skin = rest_pose(&plain);
    let mut a = SkinnedMeshBinding::new(&mut r, &plain.geometry, 2.0);
    let mut b = SkinnedMeshBinding::new(&mut r, &morphed.geometry, 2.0);
    b.set_morph_weights(&r, &[0.0, 0.0]);
    assert!(a.update_gpu(&r, &skin));
    assert!(b.update_gpu(&r, &skin));
    let (va, vb) = (vertices(&r, a.mesh_slot), vertices(&r, b.mesh_slot));
    assert_eq!(
        va.iter()
            .map(|v| (v.position, v.normal_oct))
            .collect::<Vec<_>>(),
        vb.iter()
            .map(|v| (v.position, v.normal_oct))
            .collect::<Vec<_>>()
    );
}

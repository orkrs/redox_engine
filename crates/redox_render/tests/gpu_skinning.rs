//! Specification (tests first) for GPU skinning (plan item A3).
//!
//! API to implement in `redox_render`:
//!
//! ```text
//! impl SkinnedMeshBinding {
//!     /// Skins on the GPU: uploads `skin_matrices`, runs a compute pass that blends the bind pose
//!     /// with the joint matrices and writes the QUANTISED vertices (same 16-byte `CompressedVertex`
//!     /// layout and same quantisation bounds as `Renderer::upload_mesh`) straight into the mesh
//!     /// slot's vertex buffer. No per-frame CPU deformation or vertex upload.
//!     /// Returns `false` (and leaves the mesh untouched) when `skin_matrices` has fewer matrices
//!     /// than the geometry's highest joint index needs, or the renderer cannot skin on the GPU.
//!     /// After this call `deformed_bounds()` / `swept_bounds()` return the (conservative) quantisation
//!     /// `bounds` of the binding instead of tight bounds: nothing is read back.
//!     pub fn update_gpu(&mut self, renderer: &Renderer, skin_matrices: &[Mat4]) -> bool;
//! }
//! impl Renderer {
//!     /// Blocking readback of a mesh slot's vertex buffer (tests and tools). `None` for an unknown slot.
//!     pub fn read_mesh_vertex_bytes(&self, slot: usize) -> Option<Vec<u8>>;
//! }
//! ```
//!
//! Oracle: the CPU path (`SkinnedMeshBinding::update`) on the same pose. The GPU result must match it
//! to within a couple of quantisation steps (float rounding differs between CPU and GPU). The tangent
//! stays the CPU path's default tangent (1, 0, 0) for now. The compute shader goes into
//! `crates/redox_render/shaders/` and must be registered like the other shaders (embedded list, hot
//! reload, naga validation test in `shader/mod.rs`).
//!
//! Tests are skipped when no GPU adapter exists.

#![allow(clippy::unwrap_used)]

mod common;

use redox_anim::Pose;
use redox_anim::procedural::demo_rig;
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

/// Skin matrices of every clip at `steps` evenly spaced times.
fn poses(steps: usize) -> Vec<Vec<Mat4>> {
    let model = demo_rig();
    let mut out = Vec::new();
    for clip in &model.clips {
        for step in 0..steps {
            let t = clip.duration * step as f32 / steps as f32;
            let local = clip.sample(&model.skeleton, t, true);
            out.push(Pose::from_local(&model.skeleton, &local).skin_matrices(&model.skeleton));
        }
    }
    out
}

fn max_diff_i16(a: [i16; 4], b: [i16; 4]) -> i32 {
    (0..3)
        .map(|i| (a[i] as i32 - b[i] as i32).abs())
        .max()
        .unwrap()
}

#[test]
fn gpu_skinning_matches_the_cpu_path() {
    let Some(mut r) = renderer() else { return };
    let model = demo_rig();
    let margin = 2.0;
    let mut cpu = SkinnedMeshBinding::new(&mut r, &model.geometry, margin);
    let mut gpu = SkinnedMeshBinding::new(&mut r, &model.geometry, margin);
    assert_eq!(cpu.bounds, gpu.bounds);

    for (i, skin) in poses(6).iter().enumerate() {
        assert!(cpu.update(&r, skin));
        assert!(gpu.update_gpu(&r, skin));
        let expected = vertices(&r, cpu.mesh_slot);
        let actual = vertices(&r, gpu.mesh_slot);
        assert_eq!(expected.len(), actual.len());
        for (v, (e, a)) in expected.iter().zip(&actual).enumerate() {
            // One quantisation step is 1/32767 of the box: allow 3 for CPU/GPU float differences.
            assert!(
                max_diff_i16(e.position, a.position) <= 3,
                "pose {i} vertex {v}: position {:?} vs {:?}",
                e.position,
                a.position
            );
            assert_eq!(e.position[3], a.position[3], "handedness sign");
            for k in 0..2 {
                assert!(
                    (e.normal_oct[k] as i32 - a.normal_oct[k] as i32).abs() <= 2,
                    "pose {i} vertex {v}: normal {:?} vs {:?}",
                    e.normal_oct,
                    a.normal_oct
                );
            }
            assert_eq!(e.tangent_oct, a.tangent_oct, "tangent stays the default");
            assert_eq!(e.uv, a.uv, "uv is untouched");
        }
    }
}

#[test]
fn an_unsuitable_matrix_list_is_rejected_and_the_mesh_is_untouched() {
    let Some(mut r) = renderer() else { return };
    let model = demo_rig();
    let mut gpu = SkinnedMeshBinding::new(&mut r, &model.geometry, 2.0);
    let before = vertices(&r, gpu.mesh_slot);
    assert!(!gpu.update_gpu(&r, &[]));
    assert_eq!(vertices(&r, gpu.mesh_slot), before);
}

#[test]
fn bounds_stay_conservative_after_a_gpu_update() {
    let Some(mut r) = renderer() else { return };
    let model = demo_rig();
    let mut gpu = SkinnedMeshBinding::new(&mut r, &model.geometry, 2.0);
    let skin = &poses(3)[1];
    assert!(gpu.update_gpu(&r, skin));
    assert_eq!(gpu.deformed_bounds(), gpu.bounds);
    assert_eq!(gpu.swept_bounds(), gpu.bounds);
}

#[test]
fn many_instances_can_be_skinned_in_one_frame() {
    let Some(mut r) = renderer() else { return };
    let model = demo_rig();
    let mut bindings: Vec<_> = (0..20)
        .map(|_| SkinnedMeshBinding::new(&mut r, &model.geometry, 2.0))
        .collect();
    let all = poses(4);
    for (i, b) in bindings.iter_mut().enumerate() {
        assert!(b.update_gpu(&r, &all[i % all.len()]));
    }
    // Two instances with the same pose are bit-identical; different poses differ.
    let a = vertices(&r, bindings[0].mesh_slot);
    let same = vertices(&r, bindings[all.len()].mesh_slot);
    let other = vertices(&r, bindings[1].mesh_slot);
    assert_eq!(a, same);
    assert_ne!(a, other);
}

/// Timing for the report (run with `cargo test --release -p redox_render --test gpu_skinning -- --ignored --nocapture`).
#[test]
#[ignore = "timing report, not a correctness test"]
fn timing_100_characters_cpu_vs_gpu() {
    let Some(mut r) = renderer() else { return };
    let model = demo_rig();
    let mut bindings: Vec<_> = (0..100)
        .map(|_| SkinnedMeshBinding::new(&mut r, &model.geometry, 2.0))
        .collect();
    let all = poses(8);
    for gpu in [false, true] {
        let frames = 60;
        let start = std::time::Instant::now();
        for f in 0..frames {
            for (i, b) in bindings.iter_mut().enumerate() {
                let skin = &all[(i + f) % all.len()];
                if gpu {
                    b.update_gpu(&r, skin);
                } else {
                    b.update(&r, skin);
                }
            }
        }
        // Wait for the GPU so the CPU-side submission time is not all that is measured.
        let _ = r.read_mesh_vertex_bytes(bindings[0].mesh_slot);
        let ms = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!(
            "{}: {ms:.3} ms per frame for 100 characters",
            if gpu { "GPU" } else { "CPU" }
        );
    }
}

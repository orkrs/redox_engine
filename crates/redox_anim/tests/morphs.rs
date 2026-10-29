//! Morph targets (ANM-02): CPU deformation, weights in clips, glTF import. The glTF asset is built in memory.

#![allow(clippy::unwrap_used)]

use redox_anim::gltf_import::load_glb;
use redox_anim::{AnimationClip, MorphTarget, SkinnedGeometry};
use redox_math::{Mat4, Vec3};

fn geometry_with_targets(targets: Vec<MorphTarget>) -> SkinnedGeometry {
    SkinnedGeometry {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uvs: vec![[0.0; 2]; 3],
        joints: vec![[0, 0, 0, 0]; 3],
        weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
        indices: vec![0, 1, 2],
        morph_targets: targets,
        ..Default::default()
    }
}

fn lift(y: f32) -> MorphTarget {
    MorphTarget {
        name: "lift".into(),
        positions: vec![[0.0, y, 0.0]; 3],
        ..Default::default()
    }
}

fn morph(g: &SkinnedGeometry, weights: &[f32]) -> Vec<[f32; 3]> {
    let (mut p, mut n) = (Vec::new(), Vec::new());
    g.morph_into(weights, &mut p, &mut n);
    p
}

#[test]
fn zero_weights_leave_the_bind_pose() {
    let g = geometry_with_targets(vec![lift(1.0)]);
    assert_eq!(morph(&g, &[0.0]), g.positions);
}

#[test]
fn a_full_weight_adds_the_offset() {
    let g = geometry_with_targets(vec![lift(1.0)]);
    let p = morph(&g, &[1.0]);
    assert!((p[0][1] - 1.0).abs() < 1e-6);
    assert!((p[2][1] - 2.0).abs() < 1e-6);
}

#[test]
fn weights_add_up_linearly() {
    let g = geometry_with_targets(vec![lift(1.0), lift(4.0)]);
    let p = morph(&g, &[0.5, 0.25]);
    assert!((p[0][1] - 1.5).abs() < 1e-6);
}

#[test]
fn missing_and_nan_weights_count_as_zero() {
    let g = geometry_with_targets(vec![lift(1.0)]);
    assert_eq!(morph(&g, &[]), g.positions);
    assert_eq!(morph(&g, &[f32::NAN]), g.positions);
}

#[test]
fn an_empty_normal_array_keeps_the_bind_normals() {
    let g = geometry_with_targets(vec![lift(1.0)]);
    let (mut p, mut n) = (Vec::new(), Vec::new());
    g.morph_into(&[1.0], &mut p, &mut n);
    assert_eq!(n, g.normals);
}

#[test]
fn without_targets_the_morphed_deform_equals_the_plain_one() {
    let g = geometry_with_targets(Vec::new());
    let skin = [Mat4::from_rotation_z(0.7)];
    let (mut p1, mut n1, mut p2, mut n2) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    g.deform_into(&skin, &mut p1, &mut n1);
    g.deform_morphed_into(&skin, &[1.0, 1.0], &mut p2, &mut n2);
    assert_eq!((p1, n1), (p2, n2));
}

#[test]
fn morphing_happens_before_skinning() {
    let g = geometry_with_targets(vec![MorphTarget {
        name: "x".into(),
        positions: vec![[1.0, 0.0, 0.0]; 3],
        ..Default::default()
    }]);
    let skin = [Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)];
    let (mut p, mut n) = (Vec::new(), Vec::new());
    g.deform_morphed_into(&skin, &[1.0], &mut p, &mut n);
    // Vertex 0: bind (0,0,0) + offset (1,0,0), then rotated 90 degrees about Z -> (0,1,0).
    let v = Vec3::from(p[0]);
    assert!((v - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5, "{v:?}");
}

#[test]
fn validate_rejects_a_target_of_the_wrong_length() {
    let mut g = geometry_with_targets(vec![lift(1.0)]);
    assert!(g.validate(1).is_ok());
    g.morph_targets[0].positions.pop();
    assert!(g.validate(1).is_err());
}

fn clip_with_weights() -> AnimationClip {
    AnimationClip {
        name: "m".into(),
        duration: 2.0,
        morph_weights: vec![(0.0, vec![0.0, 1.0]), (2.0, vec![1.0, 0.0])],
        ..Default::default()
    }
}

#[test]
fn a_clip_samples_morph_weights_between_keyframes() {
    let w = clip_with_weights().sample_morph_weights(1.0, false, 2);
    assert!((w[0] - 0.5).abs() < 1e-6 && (w[1] - 0.5).abs() < 1e-6);
}

#[test]
fn weights_clamp_outside_the_clip() {
    let clip = clip_with_weights();
    assert_eq!(clip.sample_morph_weights(-3.0, false, 2), vec![0.0, 1.0]);
    assert_eq!(clip.sample_morph_weights(9.0, false, 2), vec![1.0, 0.0]);
}

#[test]
fn a_looping_clip_wraps_morph_weights() {
    let clip = clip_with_weights();
    let a = clip.sample_morph_weights(0.5, true, 2);
    let b = clip.sample_morph_weights(2.5, true, 2);
    assert!((a[0] - b[0]).abs() < 1e-5);
}

#[test]
fn short_keyframes_are_padded_with_zero() {
    let clip = AnimationClip {
        duration: 1.0,
        morph_weights: vec![(0.0, vec![1.0]), (1.0, vec![1.0])],
        ..Default::default()
    };
    assert_eq!(
        clip.sample_morph_weights(0.5, false, 3),
        vec![1.0, 0.0, 0.0]
    );
}

#[test]
fn a_clip_without_morph_keys_gives_zero_weights() {
    let clip = AnimationClip::default();
    assert_eq!(clip.sample_morph_weights(0.5, false, 2), vec![0.0, 0.0]);
}

// ---- glTF import from an in-memory .glb ----

fn f32s(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// One triangle, one joint, one morph target (vertex 0 moves +0.5 in Y), default weight 0.25, and an
/// animation of the morph weight from 0 to 1 over one second.
fn morph_glb(targets: bool) -> Vec<u8> {
    let mut bin = Vec::new();
    bin.extend(f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])); // positions @0
    bin.extend(f32s(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0])); // normals @36
    bin.extend([0u8; 12]); // joints @72
    bin.extend(f32s(&[
        1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
    ])); // weights @84
    bin.extend([0u8, 0, 1, 0, 2, 0, 0, 0]); // indices @132
    bin.extend(f32s(&[
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])); // inverse bind @140
    bin.extend(f32s(&[0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])); // morph delta @204
    bin.extend(f32s(&[0.0, 1.0])); // anim times @240
    bin.extend(f32s(&[0.0, 1.0])); // morph weights @248
    assert_eq!(bin.len(), 256);

    let (target_json, mesh_weights, animation) = if targets {
        (
            r#","targets":[{"POSITION":6}]"#,
            r#","weights":[0.25]"#,
            r#","animations":[{"channels":[{"sampler":0,"target":{"node":0,"path":"weights"}}],"samplers":[{"input":7,"output":8}]}]"#,
        )
    } else {
        ("", "", "")
    };
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,1]}}],
"nodes":[{{"name":"Body","mesh":0,"skin":0}},{{"name":"root"}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1,"JOINTS_0":2,"WEIGHTS_0":3}},"indices":4{target_json}}}]{mesh_weights}}}],
"skins":[{{"joints":[1],"inverseBindMatrices":5}}],
"buffers":[{{"byteLength":256}}],
"bufferViews":[
{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":36}},
{{"buffer":0,"byteOffset":72,"byteLength":12}},{{"buffer":0,"byteOffset":84,"byteLength":48}},
{{"buffer":0,"byteOffset":132,"byteLength":6}},{{"buffer":0,"byteOffset":140,"byteLength":64}},
{{"buffer":0,"byteOffset":204,"byteLength":36}},{{"buffer":0,"byteOffset":240,"byteLength":8}},
{{"buffer":0,"byteOffset":248,"byteLength":8}}],
"accessors":[
{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
{{"bufferView":1,"componentType":5126,"count":3,"type":"VEC3"}},
{{"bufferView":2,"componentType":5121,"count":3,"type":"VEC4"}},
{{"bufferView":3,"componentType":5126,"count":3,"type":"VEC4"}},
{{"bufferView":4,"componentType":5123,"count":3,"type":"SCALAR"}},
{{"bufferView":5,"componentType":5126,"count":1,"type":"MAT4"}},
{{"bufferView":6,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[0,0.5,0]}},
{{"bufferView":7,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]}},
{{"bufferView":8,"componentType":5126,"count":2,"type":"SCALAR"}}]{animation}}}"#
    );
    let mut json = json.into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::new();
    glb.extend(b"glTF");
    glb.extend(2u32.to_le_bytes());
    glb.extend((total as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(json);
    glb.extend((bin.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(bin);
    glb
}

#[test]
fn morph_targets_are_imported_with_their_offsets() {
    let models = load_glb(&morph_glb(true)).unwrap();
    let g = &models[0].geometry;
    assert_eq!(g.morph_targets.len(), 1);
    assert_eq!(g.morph_targets[0].positions[0], [0.0, 0.5, 0.0]);
    assert!(g.validate(1).is_ok());
}

#[test]
fn default_weights_come_from_the_mesh() {
    let models = load_glb(&morph_glb(true)).unwrap();
    assert_eq!(models[0].geometry.morph_defaults, vec![0.25]);
}

#[test]
fn a_model_without_targets_has_none() {
    let models = load_glb(&morph_glb(false)).unwrap();
    assert!(models[0].geometry.morph_targets.is_empty());
}

#[test]
fn an_imported_clip_animates_the_morph_weights() {
    let models = load_glb(&morph_glb(true)).unwrap();
    let clip = &models[0].clips[0];
    assert!((clip.duration - 1.0).abs() < 1e-6);
    let w = clip.sample_morph_weights(0.5, false, 1);
    assert!((w[0] - 0.5).abs() < 1e-5, "{w:?}");
}

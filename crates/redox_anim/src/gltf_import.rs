//! glTF / GLB import of skinned meshes: skeleton, bind-pose geometry and animation clips.
//!
//! Only nodes that have both a mesh and a skin are imported. The mesh node's own transform is
//! ignored (vertices are taken in the skin's bind space). Cubic-spline channels are sampled at
//! their keyframe values, step interpolation is treated as linear.

use std::collections::HashMap;

use redox_math::{Mat4, Quat, Vec3};

use crate::{AnimationClip, Keyframe, Skeleton, SkinnedGeometry, SkinnedModel};

/// Loads every skinned mesh in a `.glb` / self-contained `.gltf` byte slice.
pub fn load_glb(bytes: &[u8]) -> Result<Vec<SkinnedModel>, String> {
    let (document, buffers, _images) = gltf::import_slice(bytes).map_err(|e| e.to_string())?;
    let mut models = Vec::new();
    for node in document.nodes() {
        if let (Some(mesh), Some(skin)) = (node.mesh(), node.skin()) {
            models.push(build_model(&document, &buffers, &mesh, &skin)?);
        }
    }
    Ok(models)
}

fn build_model(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    mesh: &gltf::Mesh,
    skin: &gltf::Skin,
) -> Result<SkinnedModel, String> {
    let joint_nodes: Vec<usize> = skin.joints().map(|n| n.index()).collect();
    let mut skeleton = build_skeleton(document, skin, &joint_nodes);

    let reader = skin.reader(|b| Some(&buffers[b.index()]));
    if let Some(ibm) = reader.read_inverse_bind_matrices() {
        for (bone, m) in skeleton.bones.iter_mut().zip(ibm) {
            bone.inv_bind = Mat4::from_cols_array_2d(&m);
        }
    }

    let geometry = build_geometry(buffers, mesh)?;
    geometry.validate(skeleton.len())?;
    let clips = build_clips(document, buffers, &skeleton, &joint_nodes);
    Ok(SkinnedModel {
        skeleton,
        clips,
        geometry,
    })
}

fn build_skeleton(document: &gltf::Document, skin: &gltf::Skin, joint_nodes: &[usize]) -> Skeleton {
    let mut parent_of: HashMap<usize, usize> = HashMap::new();
    for node in document.nodes() {
        for child in node.children() {
            parent_of.insert(child.index(), node.index());
        }
    }
    let joint_of: HashMap<usize, usize> = joint_nodes
        .iter()
        .enumerate()
        .map(|(joint, node)| (*node, joint))
        .collect();

    let mut used = std::collections::HashSet::new();
    let bones = skin
        .joints()
        .enumerate()
        .map(|(i, node)| {
            let mut name = node.name().unwrap_or("joint").to_string();
            if !used.insert(name.clone()) {
                name = format!("{name}_{i}");
                used.insert(name.clone());
            }
            let parent = nearest_joint_ancestor(node.index(), &parent_of, &joint_of);
            let local = Mat4::from_cols_array_2d(&node.transform().matrix());
            (name, parent, local)
        })
        .collect();
    Skeleton::from_local_bind(bones)
}

fn nearest_joint_ancestor(
    node: usize,
    parent_of: &HashMap<usize, usize>,
    joint_of: &HashMap<usize, usize>,
) -> Option<usize> {
    let mut current = node;
    // Bounded walk: a malformed (cyclic) hierarchy cannot hang the importer.
    for _ in 0..parent_of.len() + 1 {
        current = *parent_of.get(&current)?;
        if let Some(joint) = joint_of.get(&current) {
            return Some(*joint);
        }
    }
    None
}

fn build_geometry(
    buffers: &[gltf::buffer::Data],
    mesh: &gltf::Mesh,
) -> Result<SkinnedGeometry, String> {
    let mut g = SkinnedGeometry::default();
    for primitive in mesh.primitives() {
        let reader = primitive.reader(|b| Some(&buffers[b.index()]));
        let positions: Vec<[f32; 3]> = reader
            .read_positions()
            .ok_or("primitive has no POSITION")?
            .collect();
        let count = positions.len();
        let base = g.positions.len() as u32;

        let joints: Vec<[u16; 4]> = reader
            .read_joints(0)
            .ok_or("primitive has no JOINTS_0")?
            .into_u16()
            .collect();
        let weights: Vec<[f32; 4]> = reader
            .read_weights(0)
            .ok_or("primitive has no WEIGHTS_0")?
            .into_f32()
            .collect();
        let normals: Vec<[f32; 3]> = reader
            .read_normals()
            .map_or_else(|| vec![[0.0, 1.0, 0.0]; count], |n| n.collect());
        let uvs: Vec<[f32; 2]> = reader
            .read_tex_coords(0)
            .map_or_else(|| vec![[0.0; 2]; count], |t| t.into_f32().collect());
        let indices: Vec<u32> = reader
            .read_indices()
            .map_or_else(|| (0..count as u32).collect(), |i| i.into_u32().collect());

        if [joints.len(), weights.len(), normals.len(), uvs.len()]
            .iter()
            .any(|&n| n != count)
        {
            return Err("primitive attribute counts differ".to_string());
        }
        g.positions.extend(positions);
        g.normals.extend(normals);
        g.uvs.extend(uvs);
        g.joints.extend(joints);
        g.weights.extend(weights);
        g.indices.extend(indices.into_iter().map(|i| i + base));
    }
    Ok(g)
}

#[derive(Default)]
struct BoneChannels {
    translation: Vec<(f32, Vec3)>,
    rotation: Vec<(f32, Quat)>,
    scale: Vec<(f32, Vec3)>,
}

fn build_clips(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    skeleton: &Skeleton,
    joint_nodes: &[usize],
) -> Vec<AnimationClip> {
    let joint_of: HashMap<usize, usize> = joint_nodes
        .iter()
        .enumerate()
        .map(|(joint, node)| (*node, joint))
        .collect();
    let mut clips = Vec::new();
    for (index, animation) in document.animations().enumerate() {
        let mut channels: HashMap<usize, BoneChannels> = HashMap::new();
        let mut duration = 0.0f32;
        for channel in animation.channels() {
            let Some(&joint) = joint_of.get(&channel.target().node().index()) else {
                continue;
            };
            let reader = channel.reader(|b| Some(&buffers[b.index()]));
            let (Some(times), Some(outputs)) = (reader.read_inputs(), reader.read_outputs()) else {
                continue;
            };
            let times: Vec<f32> = times.collect();
            duration = duration.max(times.last().copied().unwrap_or(0.0));
            let cubic =
                channel.sampler().interpolation() == gltf::animation::Interpolation::CubicSpline;
            let bone = channels.entry(joint).or_default();
            match outputs {
                gltf::animation::util::ReadOutputs::Translations(v) => {
                    bone.translation = pair(&times, pick(v.map(Vec3::from).collect(), cubic));
                }
                gltf::animation::util::ReadOutputs::Rotations(r) => {
                    let q = r
                        .into_f32()
                        .map(|q| Quat::from_array(q).normalize())
                        .collect();
                    bone.rotation = pair(&times, pick(q, cubic));
                }
                gltf::animation::util::ReadOutputs::Scales(v) => {
                    bone.scale = pair(&times, pick(v.map(Vec3::from).collect(), cubic));
                }
                gltf::animation::util::ReadOutputs::MorphTargetWeights(_) => {}
            }
        }

        let mut clip = AnimationClip {
            name: animation
                .name()
                .map_or_else(|| format!("clip_{index}"), str::to_string),
            duration,
            ..Default::default()
        };
        for (joint, data) in channels {
            clip.tracks.insert(
                skeleton.bones[joint].name.clone(),
                keyframes(skeleton, joint, &data),
            );
        }
        clips.push(clip);
    }
    clips
}

/// Cubic-spline outputs come as (in-tangent, value, out-tangent) triples: keep the value.
fn pick<T: Copy>(values: Vec<T>, cubic: bool) -> Vec<T> {
    if cubic {
        values.chunks_exact(3).map(|c| c[1]).collect()
    } else {
        values
    }
}

fn pair<T>(times: &[f32], values: Vec<T>) -> Vec<(f32, T)> {
    times.iter().copied().zip(values).collect()
}

fn keyframes(skeleton: &Skeleton, joint: usize, data: &BoneChannels) -> Vec<Keyframe> {
    let bind = crate::BoneTransform::from_mat4(skeleton.bones[joint].bind_pose);
    let mut times: Vec<f32> = data
        .translation
        .iter()
        .map(|k| k.0)
        .chain(data.rotation.iter().map(|k| k.0))
        .chain(data.scale.iter().map(|k| k.0))
        .collect();
    times.sort_by(f32::total_cmp);
    times.dedup();
    times
        .into_iter()
        .map(|time| Keyframe {
            time,
            translation: sample(&data.translation, time, bind.translation, Vec3::lerp),
            rotation: sample(&data.rotation, time, bind.rotation, |a, b, t| a.slerp(b, t)),
            scale: sample(&data.scale, time, bind.scale, Vec3::lerp),
        })
        .collect()
}

fn sample<T: Copy>(keys: &[(f32, T)], time: f32, default: T, lerp: impl Fn(T, T, f32) -> T) -> T {
    let next = keys.partition_point(|k| k.0 <= time);
    match (next.checked_sub(1).map(|i| keys[i]), keys.get(next)) {
        (None, None) => default,
        (None, Some(b)) => b.1,
        (Some(a), None) => a.1,
        (Some(a), Some(b)) => lerp(
            a.1,
            b.1,
            ((time - a.0) / (b.0 - a.0).max(f32::EPSILON)).clamp(0.0, 1.0),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pose;

    fn bytes_f32(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// One triangle skinned to two joints (root, tip) with a 90 degree tip rotation clip.
    fn sample_glb() -> Vec<u8> {
        let mut bin = Vec::new();
        bin.extend(bytes_f32(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2, 0.5, 0.0])); // positions @0
        bin.extend(bytes_f32(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0])); // normals @36
        bin.extend([0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0]); // joints u8 @72
        bin.extend(bytes_f32(&[
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0,
        ])); // weights @84
        bin.extend([0u8, 0, 1, 0, 2, 0, 0, 0]); // indices u16 + pad @132
        #[rustfmt::skip]
        bin.extend(bytes_f32(&[
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 1.0,
        ])); // inverse bind matrices @140
        bin.extend(bytes_f32(&[0.0, 1.0])); // anim times @268
        let h = std::f32::consts::FRAC_1_SQRT_2;
        bin.extend(bytes_f32(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, h, h])); // rotations @276
        assert_eq!(bin.len(), 308);

        let json = r#"{
          "asset": {"version": "2.0"},
          "scene": 0,
          "scenes": [{"nodes": [0, 1]}],
          "nodes": [
            {"name": "Body", "mesh": 0, "skin": 0},
            {"name": "root", "children": [2]},
            {"name": "tip", "translation": [0, 1, 0]}
          ],
          "meshes": [{"primitives": [{"attributes":
            {"POSITION": 0, "NORMAL": 1, "JOINTS_0": 2, "WEIGHTS_0": 3}, "indices": 4}]}],
          "skins": [{"joints": [1, 2], "inverseBindMatrices": 5}],
          "animations": [{"name": "bend",
            "samplers": [{"input": 6, "output": 7, "interpolation": "LINEAR"}],
            "channels": [{"sampler": 0, "target": {"node": 2, "path": "rotation"}}]}],
          "buffers": [{"byteLength": 308}],
          "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 36},
            {"buffer": 0, "byteOffset": 72, "byteLength": 12},
            {"buffer": 0, "byteOffset": 84, "byteLength": 48},
            {"buffer": 0, "byteOffset": 132, "byteLength": 6},
            {"buffer": 0, "byteOffset": 140, "byteLength": 128},
            {"buffer": 0, "byteOffset": 268, "byteLength": 8},
            {"buffer": 0, "byteOffset": 276, "byteLength": 32}
          ],
          "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
             "min": [0, 0, 0], "max": [0.2, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5121, "count": 3, "type": "VEC4"},
            {"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"},
            {"bufferView": 4, "componentType": 5123, "count": 3, "type": "SCALAR"},
            {"bufferView": 5, "componentType": 5126, "count": 2, "type": "MAT4"},
            {"bufferView": 6, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
            {"bufferView": 7, "componentType": 5126, "count": 2, "type": "VEC4"}
          ]
        }"#;
        glb(json, &bin)
    }

    fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend(b"glTF");
        out.extend(2u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(&json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(&bin);
        out
    }

    #[test]
    fn imports_skeleton_geometry_and_clip() {
        let models = load_glb(&sample_glb()).expect("valid GLB");
        assert_eq!(models.len(), 1);
        let model = &models[0];

        assert_eq!(model.skeleton.len(), 2);
        assert_eq!(model.skeleton.bones[0].name, "root");
        assert_eq!(model.skeleton.bones[1].parent, Some(0));
        let ibm_tip = model.skeleton.bones[1].inv_bind.transform_point3(Vec3::Y);
        assert!(
            ibm_tip.length() < 1.0e-5,
            "inverse bind moves tip origin back to zero"
        );

        assert_eq!(model.geometry.vertex_count(), 3);
        assert_eq!(model.geometry.indices, vec![0, 1, 2]);
        assert_eq!(model.geometry.joints[2], [0, 1, 0, 0]);
        assert!((model.geometry.weights[2][0] - 0.5).abs() < 1.0e-6);

        assert_eq!(model.clips.len(), 1);
        assert_eq!(model.clips[0].name, "bend");
        assert_eq!(model.clips[0].duration, 1.0);
        assert_eq!(model.clips[0].tracks["tip"].len(), 2);
    }

    #[test]
    fn imported_clip_bends_the_skinned_vertex() {
        let model = load_glb(&sample_glb()).unwrap().remove(0);
        let local = model.clips[0].sample(&model.skeleton, 1.0, false);
        let pose = Pose::from_local(&model.skeleton, &local);
        let (mut p, mut n) = (Vec::new(), Vec::new());
        model
            .geometry
            .deform_into(&pose.skin_matrices(&model.skeleton), &mut p, &mut n);
        // Vertex 1 is fully on the tip joint at (0,1,0): rotating it 90 deg about Z around its
        // own origin leaves it in place.
        assert!(Vec3::from(p[1]).distance(Vec3::Y) < 1.0e-4);
        // Vertex 0 is on the root and does not move.
        assert!(Vec3::from(p[0]).length() < 1.0e-5);
        // Vertex 2 is half on the tip joint, so it must have moved.
        assert!(Vec3::from(p[2]).distance(Vec3::new(0.2, 0.5, 0.0)) > 0.05);
    }

    #[test]
    fn garbage_input_is_an_error() {
        assert!(load_glb(b"not a glb").is_err());
    }
}

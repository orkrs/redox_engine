//! Procedural rigs for demos, tests and the editor AnimGraph preview.

use std::f32::consts::TAU;

use redox_math::{Mat4, Quat, Vec3};

use crate::{AnimationClip, Keyframe, Skeleton, SkinnedGeometry, SkinnedModel};

const RINGS_PER_SEGMENT: usize = 4;
/// Fraction of the base radius lost towards the tip.
const TAPER: f32 = 0.55;

/// A tapered cylinder along +Y skinned to `segments + 1` bones spaced `segment_len` apart.
///
/// Bone `i` sits at `y = i * segment_len`; vertices blend linearly between neighbouring bones,
/// so rotating bone `i` bends everything above it.
pub fn chain_model(segments: usize, segment_len: f32, radius: f32, sides: usize) -> SkinnedModel {
    let segments = segments.max(1);
    let sides = sides.max(3);

    let skeleton = Skeleton::from_local_bind(
        (0..=segments)
            .map(|i| {
                let local = if i == 0 { Mat4::IDENTITY } else { Mat4::from_translation(Vec3::Y * segment_len) };
                (format!("bone_{i}"), i.checked_sub(1), local)
            })
            .collect(),
    );

    let rings = segments * RINGS_PER_SEGMENT + 1;
    let total = segments as f32 * segment_len;
    let mut geometry = SkinnedGeometry::default();
    for ring in 0..rings {
        let y = ring as f32 * segment_len / RINGS_PER_SEGMENT as f32;
        let along = y / segment_len;
        let bone = (along.floor() as usize).min(segments - 1);
        let blend = along - bone as f32;
        let r = radius * (1.0 - TAPER * y / total);
        for side in 0..sides {
            let theta = TAU * side as f32 / sides as f32;
            let (sin, cos) = theta.sin_cos();
            geometry.positions.push([r * cos, y, r * sin]);
            geometry.normals.push([cos, 0.0, sin]);
            geometry.uvs.push([side as f32 / sides as f32, y / total]);
            geometry.joints.push([bone as u16, bone as u16 + 1, 0, 0]);
            geometry.weights.push([1.0 - blend, blend, 0.0, 0.0]);
        }
    }
    for ring in 0..rings - 1 {
        for side in 0..sides {
            let next = (side + 1) % sides;
            let a = (ring * sides + side) as u32;
            let b = (ring * sides + next) as u32;
            let c = ((ring + 1) * sides + side) as u32;
            let d = ((ring + 1) * sides + next) as u32;
            geometry.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let clips = demo_clips(&skeleton, segment_len);
    SkinnedModel { skeleton, clips, geometry }
}

/// Three looping clips ("Sway", "Wave", "Coil") that bend every bone with a travelling phase.
pub fn demo_clips(skeleton: &Skeleton, segment_len: f32) -> Vec<AnimationClip> {
    [
        ("Sway", 0.22, 0.5, 3.0),
        ("Wave", 0.38, 0.9, 1.6),
        ("Coil", 0.52, 1.3, 2.4),
    ]
    .into_iter()
    .map(|(name, amplitude, phase_step, duration)| {
        wave_clip(name, skeleton, segment_len, amplitude, phase_step, duration)
    })
    .collect()
}

fn wave_clip(
    name: &str,
    skeleton: &Skeleton,
    segment_len: f32,
    amplitude: f32,
    phase_step: f32,
    duration: f32,
) -> AnimationClip {
    const KEYS_PER_LOOP: usize = 12;
    let mut clip = AnimationClip { name: name.into(), duration, ..Default::default() };
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let translation = if i == 0 { Vec3::ZERO } else { Vec3::Y * segment_len };
        // Last key repeats the first so the loop is seamless.
        let keys = (0..=KEYS_PER_LOOP)
            .map(|k| {
                let phase = TAU * k as f32 / KEYS_PER_LOOP as f32 - i as f32 * phase_step;
                Keyframe {
                    time: duration * k as f32 / KEYS_PER_LOOP as f32,
                    translation,
                    rotation: Quat::from_rotation_z(amplitude * phase.sin())
                        * Quat::from_rotation_x(0.6 * amplitude * phase.cos()),
                    scale: Vec3::ONE,
                }
            })
            .collect();
        clip.tracks.insert(bone.name.clone(), keys);
    }
    clip
}

/// The rig shown in the editor AnimGraph and the `skinned_demo` example.
pub fn demo_rig() -> SkinnedModel {
    chain_model(5, 0.5, 0.16, 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pose;

    #[test]
    fn chain_geometry_is_consistent() {
        let model = chain_model(3, 0.5, 0.1, 8);
        assert_eq!(model.skeleton.len(), 4);
        model.geometry.validate(model.skeleton.len()).expect("valid geometry");
        assert_eq!(model.clips.len(), 3);
        assert_eq!(model.geometry.indices.len() % 3, 0);
    }

    #[test]
    fn bind_pose_reproduces_bind_geometry() {
        let model = chain_model(3, 0.5, 0.1, 8);
        let pose = Pose::from_local(&model.skeleton, &model.skeleton.bind_local_pose());
        let (mut p, mut n) = (Vec::new(), Vec::new());
        model.geometry.deform_into(&pose.skin_matrices(&model.skeleton), &mut p, &mut n);
        for (a, b) in p.iter().zip(&model.geometry.positions) {
            assert!(Vec3::from(*a).distance(Vec3::from(*b)) < 1.0e-5);
        }
    }

    #[test]
    fn clips_loop_seamlessly() {
        let model = chain_model(3, 0.5, 0.1, 8);
        for clip in &model.clips {
            let start = clip.sample(&model.skeleton, 0.0, true);
            let end = clip.sample(&model.skeleton, clip.duration - 1.0e-4, true);
            for (a, b) in start.bones.iter().zip(&end.bones) {
                assert!(a.rotation.abs_diff_eq(b.rotation, 1.0e-2), "{} does not loop", clip.name);
            }
        }
    }

    #[test]
    fn animation_moves_the_tip() {
        let model = chain_model(4, 0.5, 0.1, 8);
        let tip = model.skeleton.len() - 1;
        let at = |t: f32| {
            let local = model.clips[1].sample(&model.skeleton, t, true);
            Pose::from_local(&model.skeleton, &local).bone_transforms[tip].w_axis.truncate()
        };
        assert!(at(0.0).distance(at(0.4)) > 0.05, "tip must move during the clip");
    }
}

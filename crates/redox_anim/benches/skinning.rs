//! Animation micro-benchmarks: `cargo bench -p redox_anim`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use redox_anim::SkinnedGeometry;
use redox_anim::skeleton::{AnimationClip, Keyframe, Pose, Skeleton};
use redox_math::{Mat4, Quat, Vec3};

const BONES: usize = 64;
const KEYS_PER_TRACK: usize = 60;
const VERTICES: usize = 50_000;

fn skeleton() -> Skeleton {
    let bones = (0..BONES)
        .map(|i| {
            let parent = i.checked_sub(1);
            (
                format!("bone{i}"),
                parent,
                Mat4::from_translation(Vec3::new(0.0, 0.1, 0.0)),
            )
        })
        .collect();
    Skeleton::from_local_bind(bones)
}

fn clip() -> AnimationClip {
    let mut clip = AnimationClip {
        name: "wave".into(),
        duration: 2.0,
        ..Default::default()
    };
    for i in 0..BONES {
        let keys = (0..KEYS_PER_TRACK)
            .map(|k| {
                let t = k as f32 / (KEYS_PER_TRACK - 1) as f32;
                Keyframe {
                    time: t * 2.0,
                    translation: Vec3::new(0.0, 0.1, 0.0),
                    rotation: Quat::from_rotation_z(t * 0.5 + i as f32 * 0.01),
                    scale: Vec3::ONE,
                }
            })
            .collect();
        clip.tracks.insert(format!("bone{i}"), keys);
    }
    clip
}

fn geometry() -> SkinnedGeometry {
    let mut g = SkinnedGeometry::default();
    for v in 0..VERTICES {
        let b = (v * 7) % (BONES - 1);
        g.positions
            .push([(v % 100) as f32 * 0.01, (v / 100) as f32 * 0.01, 0.0]);
        g.normals.push([0.0, 0.0, 1.0]);
        g.uvs.push([0.0, 0.0]);
        g.joints.push([b as u16, b as u16 + 1, 0, 0]);
        g.weights.push([0.6, 0.4, 0.0, 0.0]);
    }
    g.indices = (0..VERTICES as u32 - 2).collect();
    g
}

fn bench_sampling(c: &mut Criterion) {
    let (skeleton, clip) = (skeleton(), clip());
    c.bench_function("clip_sample_64_bones", |b| {
        let mut t = 0.0;
        b.iter(|| {
            t += 0.016;
            black_box(clip.sample(&skeleton, t, true))
        })
    });
    let local = clip.sample(&skeleton, 0.7, true);
    c.bench_function("pose_skin_matrices_64_bones", |b| {
        b.iter(|| {
            let pose = Pose::from_local(&skeleton, black_box(&local));
            black_box(pose.skin_matrices(&skeleton))
        })
    });
}

fn bench_deform(c: &mut Criterion) {
    let (skeleton, clip, geometry) = (skeleton(), clip(), geometry());
    let pose = Pose::from_local(&skeleton, &clip.sample(&skeleton, 0.7, true));
    let skin = pose.skin_matrices(&skeleton);
    let (mut positions, mut normals) = (Vec::new(), Vec::new());
    c.bench_function("deform_into_50k_vertices", |b| {
        b.iter(|| {
            geometry.deform_into(black_box(&skin), &mut positions, &mut normals);
            black_box(positions.len())
        })
    });
}

criterion_group!(benches, bench_sampling, bench_deform);
criterion_main!(benches);

//! Virtual-geometry asset building: `cargo bench -p redox_render --bench meshlets`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use redox_render::mesh::{Mesh, Vertex};
use redox_render::virtual_geometry::{LodChainConfig, build_vg_asset};

/// Flat grid of `n x n` quads (`2 n^2` triangles) with a gentle wave.
fn grid(n: u32) -> Mesh {
    let side = n + 1;
    let mut vertices = Vec::with_capacity((side * side) as usize);
    for z in 0..side {
        for x in 0..side {
            let (fx, fz) = (x as f32 * 0.1, z as f32 * 0.1);
            vertices.push(Vertex {
                position: [fx, (fx * 3.0).sin() * 0.2 + (fz * 2.0).cos() * 0.2, fz],
                normal: [0.0, 1.0, 0.0],
                uv: [x as f32 / n as f32, z as f32 / n as f32],
                tangent: [1.0, 0.0, 0.0],
            });
        }
    }
    let mut indices = Vec::with_capacity((n * n * 6) as usize);
    for z in 0..n {
        for x in 0..n {
            let i = z * side + x;
            indices.extend_from_slice(&[i, i + side, i + 1, i + 1, i + side, i + side + 1]);
        }
    }
    Mesh::new(vertices, indices)
}

fn bench_build(c: &mut Criterion) {
    let config = LodChainConfig::default();
    let mut group = c.benchmark_group("build_vg_asset");
    group.sample_size(10);
    for n in [64u32, 128] {
        let mesh = grid(n);
        group.bench_function(
            format!("{}k_triangles", mesh.indices.len() / 3 / 1000),
            |b| b.iter(|| black_box(build_vg_asset(black_box(&mesh), &config, 0))),
        );
    }
    group.finish();
}

criterion_group!(benches, bench_build);
criterion_main!(benches);

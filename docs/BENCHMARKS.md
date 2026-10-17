# Benchmarks

Run: `cargo bench -p redox_ecs --bench world`. Numbers are from the developer machine
(Windows 11, release profile, 2026-10-17) and are a baseline for regressions, not a promise.

| Benchmark | Time |
|---|---|
| `spawn_bundle_100k` (two components per entity) | ~17.5 ms |
| `iter_100k` (`Query<(&Position, &Velocity)>`, sequential) | ~0.64 ms |
| `par_iter_100k` (same query, rayon) | ~0.20 ms |
| `add_component_10k` (archetype migration) | ~1.5 ms |

Animation: `cargo bench -p redox_anim` (64 bones, 60 keys per track, 50 000 vertices):

| Benchmark | Time |
|---|---|
| `clip_sample_64_bones` | ~4.3 us |
| `pose_skin_matrices_64_bones` | ~1.4 us |
| `deform_into_50k_vertices` (CPU skinning, 2 bones per vertex) | ~0.53 ms |

Virtual geometry: `cargo bench -p redox_render --bench meshlets` (`build_vg_asset`, default 3-level LOD chain):

| Benchmark | Time |
|---|---|
| 8k triangles | ~8.8 ms |
| 32k triangles | ~37 ms |

Textures: `cargo bench -p redox_asset --bench ktx2` (RGBA8 KTX2 file, whole file decoded):

| Benchmark | Time |
|---|---|
| 1024 px, stored | ~272 us |
| 1024 px, Zstd | ~2.9 ms |
| 4096 px, stored | ~4.1 ms |
| 4096 px, Zstd | ~42.9 ms |

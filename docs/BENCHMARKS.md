# Benchmarks

Run: `cargo bench -p redox_ecs --bench world`. Numbers are from the developer machine
(Windows 11, release profile, 2026-10-17) and are a baseline for regressions, not a promise.

| Benchmark | Time |
|---|---|
| `spawn_bundle_100k` (two components per entity) | ~17.5 ms |
| `iter_100k` (`Query<(&Position, &Velocity)>`, sequential) | ~0.64 ms |
| `par_iter_100k` (same query, rayon) | ~0.20 ms |
| `add_component_10k` (archetype migration) | ~1.5 ms |

Not yet covered: `Animator::sample`, `SkinnedGeometry::deform_into`, meshlet building
(roadmap A10.4).

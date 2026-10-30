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

## GPU/CPU skinning threshold (ANM-04)

Measured 2026-10-30, release, RTX 4060 Ti (`cargo test --release -p redox_render --test gpu_skinning threshold_bench -- --ignored --nocapture`). Milliseconds per frame:

| vertices | characters | CPU | GPU, one submit each | GPU, one batch |
|---:|---:|---:|---:|---:|
| 500 | 1 | 0.076 | 0.071 | 0.036 |
| 500 | 10 | 0.256 | 0.377 | 0.068 |
| 500 | 100 | 2.479 | 2.666 | 0.238 |
| 2 000 | 1 | 0.099 | 0.027 | 0.023 |
| 2 000 | 10 | 0.907 | 0.367 | 0.063 |
| 2 000 | 100 | 11.738 | 3.635 | 0.305 |
| 10 000 | 1 | 0.501 | 0.034 | 0.030 |
| 10 000 | 10 | 4.987 | 0.287 | 0.054 |
| 10 000 | 100 | 51.608 | 3.245 | 0.566 |
| 50 000 | 1 | 2.982 | 0.049 | 0.050 |
| 50 000 | 10 | 31.230 | 0.402 | 0.227 |
| 50 000 | 100 | 262.608 | 3.800 | 2.134 |

Reading: at 500 vertices one mesh costs the same on both paths; from about 1 000 vertices the GPU wins even with a submit per mesh; for small meshes the GPU only wins when they are skinned together in one batch (`update_gpu_batch`). Hence `GPU_SKIN_MIN_VERTICES = 1024` and `GPU_BATCH_MIN_MESHES = 4`; `MORPH_COST_FACTOR = 2` is an estimate (the benchmark has no morph case).

## VG-02: выбор кластеров DAG на GPU (2026-10-30)

Тест `dag_bench_thousand_spheres` (`crates/redox_render/tests/vg_dag_frame.rs`, `--release -- --ignored --nocapture`): 1000 экземпляров сферы 99 856 треугольников (DAG строится 1,6 с), окно 512×512, порог ошибки 1 px, камера на расстоянии 30 м, 5 прогревочных и 30 замеряемых кадров. Отрисовано 56 228 кластеров.

| Что | Время |
|---|---:|
| GPU: `cluster_vg_cull` (в него входит обход DAG; отдельной метки у обхода нет) | 0,28 мс |
| GPU: `vg_visibility_resolve` | 1,03 мс |
| GPU: весь кадр по меткам | 1,61 мс |
| Стена: кадр от `render_frame_view` до `poll(wait)` | 12,75 мс |

Цель проход выбора ≤ 1 мс: по метке `cluster_vg_cull` (0,28 мс) достигнута; точное время обхода без отдельной метки не выделено. Расхождение «GPU 1,6 мс» и «стена 12,75 мс» означает, что кадр упирается в CPU (запись команд, загрузка данных экземпляров); причина не искалась. Замер на машине разработчика, не на эталонной станции; по умолчанию `VGConfig` мал для такого ассета (нужно `max_vertices` 1<<20, `max_indices` 1<<22), без этого `register_dag_asset` возвращает `None`.
## Motion blur spreading (PST-03)

Cost of the two tile compute passes and the spreading blur, `tests/motion_blur_spread.rs::spreading_cost_per_frame` (`#[ignore]`, 128×128, wall time per frame including the readback). Run: `cargo test -p redox_render --test motion_blur_spread -- --ignored --nocapture`.

| Spreading | ms per frame | Notes |
|---|---|---|
| off | not measured | run the ignored test on the owner's GPU |
| on | not measured | |

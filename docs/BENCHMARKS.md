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
## TSR upscale (PST-07)

Bench: `cargo test -p redox_render --test tsr -- --ignored --nocapture` (1080 and 2160 square targets, scales 1.0 / 0.77 / 0.67 / 0.5, wall-clock ms per frame including readback, PSNR to the native frame). Numbers are not recorded yet; the coordinator fills this table after the first run.

## VG-04: HLOD, 4000 объектов в 40 ячейках (2026-10-30)

Тест `four_thousand_objects_in_forty_cells_seen_from_300m` (`crates/redox_core/tests/hlod_measure.rs`, `--release -- --ignored --nocapture`). Камера на ~300 м.

| Режим | Вызовов отрисовки | `hlod_frame`, CPU |
|---|---:|---:|
| без HLOD | 4000 | 92 мкс |
| с HLOD | 40 | 152 мкс |

Число вызовов падает в 100 раз; GPU-время кадра не измерено (нужна сцена с загруженными прокси, загрузка `.hlod` при старте игры не подключена).

## RND-08 ворота (шаг 3 «два слоя страниц»)

`wave7_gpu::shadow_gate_walking_hero_cost` (ignored, release): пол + 40 статичных кубов + «герой» 0,5×1,8×0,5, идущий 120 кадров; кадр 128², VSM включён. Среднее грязных страниц 2,17 за кадр (порог 12); проходы теней (суммарно по меткам «shadow») 0,042 мс за кадр (порог 1,5 мс). Вывод: шаг 3 не нужен. Оговорка: синтетическая сцена, не Courier/Foxfire на 1080p; перепроверить на реальной сцене при жалобах.
## RND-10 шаг 1: стоимость прохода скорости объектов

Тест `object_velocity_pass_cost_table` (`crates/redox_render/tests/velocity_cost.rs`, `--release -- --ignored --nocapture`, 1024 квадрат, TAA включён). Сцена: 20 движущихся сфер по 5151 вершине, 3000 статических кубов, 50 прозрачных. Ворота: слияние при выигрыше ≥ 0,3 мс GPU или ≥ 15 % вызовов отрисовки.

| Режим | GPU, мс | CPU, мс | Вызовов forward | Вызовов скорости объектов | Всего |
|---|---:|---:|---:|---:|---:|
| (а) как сейчас | 1,902 | 3,908 | 3 | 20 | 23 |
| (б) без прохода скорости объектов | 1,878 | 3,649 | 3 | 0 | 3 |

Выигрыш: GPU 0,024 мс, CPU 0,26 мс, вызовов отрисовки −87 % (20 из 23). Формально ворота пройдены по вызовам (≥ 15 %), но абсолютный выигрыш мал: на этой сцене GPU почти не меняется. Решение о шагах 2–3 (скорость вторым выводом основного прохода) за владельцем; рекомендация — не делать, пока CPU-стоимость вызовов не станет узким местом.

## GI-01 G1: стоимость GI (заготовка, цифры — координатор)

Тест `gi_cost_by_quality` (`crates/redox_render/tests/gi_bench.rs`, `cargo test -p redox_render --test gi_bench --release -- --ignored --nocapture`). Сцена: комната Cornell и 3000 статических коробок на полу, 1080×1080, GPU-таймеры проходов `gi*` (трассировка и обновление атласов), среднее за 90 кадров.

| Качество | Лучей | Каскадов | Время GI, мс (GPU) | Цель |
|---|---:|---:|---:|---|
| High | 128 | 2 | _не измерено_ | — |
| Medium | 64 | 2 | _не измерено_ | ≤ 2,5 (ворота A7) |
| Low | 32 | 1 | _не измерено_ | — |

Если Medium выше 2,5 мс, в отчёте указывается число, без оптимизации в этом шаге.

## Граф кадра: память транзиентных целей (RND-09 шаг 5)

Вычислено по объявлению `frame_graph(1920, 1080)`, замера не было: этот шаг не запускал кадры и бенчмарки.

| Транзиент | Формат | Байты при 1080p |
|---|---|---:|
| `vg_clusters` | R32Uint | 8 294 400 |
| `depth` | Depth32Float | 8 294 400 |
| `normals` | Rgba8Unorm | 8 294 400 |
| `hiz` | R32Float | 8 294 400 |
| `ao` | R8Unorm | 2 073 600 |
| `hdr_color` | Rgba16Float | 16 588 800 |
| **Итого логических = физических (6 слотов)** | | **51 840 000** |

Экономия алиасингом: 0 байт (0 %). У шести транзиентов шесть разных форматов, а правило совместимости шага 3 требует равного формата. Порог 10 % не достигнут: алиасинг не вводится (⛔ по замеру). Ping-pong цели пост-эффектов и цели вне графа здесь не учтены.

## VG-03 ворота памяти: нужен ли стриминг (2026-10-30)

Ворота шага 3 спеки: стриминг нужен, только если память VG не помещается в бюджет. Бюджет — `stream_pool_mib` = 256 МиБ (значение по умолчанию из спеки VG-03, шаг 5). Замер: `VGSystem::gpu_bytes` (ёмкость буферов, выделяемых при `init_virtual_geometry`), считано по размерам структур: `CompressedVertex` 16 Б, индекс 4 Б, `VGInstanceData` 112 Б, `MeshletDescriptor` 64 Б. Фактическое заполнение (watermark) наружу не выводится; ёмкость — верхняя граница.

| Конфигурация | Вершины | Индексы | Экземпляры | Меши | Итого `gpu_bytes` |
|---|---:|---:|---:|---:|---:|
| `VGConfig::default()` | 4 МиБ | 2 МиБ | 0,44 МиБ | 4 МиБ | 10,44 МиБ |
| сцена `dag_bench_thousand_spheres` (1000 сфер, 99 856 треугольников) | 16 МиБ | 16 МиБ | 0,44 МиБ | 16 МиБ | 48,44 МиБ |

Буферы обхода DAG (`DagState`: до 2^16 кластеров, 2^17 child id, очереди 2^15) в `gpu_bytes` не входят. Оценка по константам с размером кластера 96 Б (из спеки VG-02): ≈ 6,5 МиБ, итого верхняя граница ≈ 55 МиБ = 22 % бюджета.

Текущие сцены (Courier, Foxfire, примеры) не вызывают `init_virtual_geometry` вне тестов, VG в них не выделяется: память VG = 0.

Вывод: ⛔ стриминг не нужен при текущих сценах. Время загрузки не замерялось: DAG строится офлайн (1,6 с на сферу 100k треугольников), в рантайме загрузки ассета по сцене нет. Пересмотреть, если появится сцена с DAG-ассетами суммарно более ~200 МиБ или загрузка станет заметной.


## GI-02 G2: стоимость карточек (заготовка, цифры — координатор)

Тест `gi_cards_cost_for_200_static_objects` (`crates/redox_render/tests/gi_cards_bench.rs`, `cargo test -p redox_render --test gi_cards_bench --release -- --ignored --nocapture`). Сцена: 200 статических кубов, карточки на 0,25 м, SDF 128³, две каскады Medium (64 луча). Печатаются размер атласа карточек и время кадра с карточками и без них (CPU-часы вокруг отправки и ожидания; для пассов — GPU-таймеры `profiler.rs`).

| Показатель | Значение |
|---|---|
| Атлас карточек, МБ | _не измерено_ (расчёт: 2048² × 16 Б = 64 МБ) |
| `gi_card_light` (1/4 атласа), мс (GPU) | _не измерено_ |
| `gi_trace` с карточками, мс (GPU) | _не измерено_ |
| `gi_trace` без карточек (GI-01), мс (GPU) | _не измерено_ |
| Сумма GI-01 + GI-02, Medium, мс | _не измерено_; цель ≤ 2,5 (ворота A7) |

Если сумма выше 2,5 мс, по закрытию GI-02 снижаем число карточек (меньше текселей на грань) и частоту обновления света (1/4 → 1/8 строк), время не растим. Ожидание до замера: карточки добавляют к трассировке чтение трёх граней (три выборки и одна загрузка глубины на попадание), освещение — до 32 шагов SDF на тексель в обновляемой четверти атласа.

## Замеры финальной волны (2026-10-30, release, RTX 4060 Ti, 1080p)

| Что | Результат | Цель |
|---|---|---|
| GI-01: проходы `gi*`, Cornell + 3000 боксов, High / Medium / Low | 2,735 / 1,421 / 0,788 мс | Medium ≤ 2,5 мс ✅ |
| GI-02: карточки + пробы против одних проб, 200 объектов, 2 каскада (wall clock) | 0,336 против 0,258 мс (+0,08 мс) | GI-01 + GI-02 на Medium укладываются в 2,5 мс ✅ |
| VG-02: `cluster_vg_cull`, 1000 сфер (99 856 треугольников), DAG-путь | 0,29 мс; всего GPU-проходы кадра 1,74 мс; память VG 50,8 МБ | ≤ 1 мс ✅ |

Атлас карточек: 64 МиБ, таблица 200 записей, 0 отброшенных граней.


## SSS: стоимость блюра (MAT-05 шаг 4, 2026-10-30, release, 1080p)

Сфера на пол-экрана с SSS-материалом, TAA включён, среднее за 60 кадров (после 10 прогревочных). Блюр включён по умолчанию (`set_sss` не вызывается), для сравнения выключен `set_sss(None)`. Тест `sss_blur_cost_at_1080p` в `tests/sss_gpu.rs`.

| Прогон | Блюр вкл.: GPU, мс | Блюр выкл.: GPU, мс | Разница GPU, мс | Разница wall, мс |
|---|---|---|---|---|
| 1 | 3,201 | 2,630 | 0,571 | 0,622 |
| 2 | 2,989 | 2,519 | 0,470 | 0,402 |

Два прохода (`sss_blur_horizontal`, `sss_blur_vertical`) не имеют GPU-меток, поэтому стоимость каждого прохода отдельно не измерена. Оценка по кадру: 0,47–0,57 мс GPU, 0,40–0,62 мс wall. Цель ≤ 0,5 мс: на границе, между прогонами разброс больше 0,1 мс. ✅ / ⛔ не назначено (решает координатор).

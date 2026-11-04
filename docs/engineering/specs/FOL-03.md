# FOL-03 — Растительность, E2: GPU-управляемые слои (трава, кусты, папоротник)

- **Статус:** не начат. **Ворота:** выполнять только если замер FOL-01 шаг 2 показал, что CPU-путь не вытягивает цель (см. «Цель»).
- **Размер:** 6 шагов (M, M, M, S, M, S) ≈ 8–12 дней. **Крейт:** `redox_render` (+ `redox_scene` шаг 5). **Зависит от:** FOL-01 (замер), MAT-08 (двусторонние материалы, режимы альфы). **Блокирует:** ничего.
- **Модель:** haiku по шагам, с проверкой координатора после каждого; шаги 2–3 вероятнее всего потребуют доработки координатором.
- Источник: `../FOREST_SCENE_FINDINGS.md`, E8, E9; план 5.1.4–5.1.5.

## Цель
200 000 лезвий травы (около 40 000 пучков в видимой зоне) при 60 кадрах/с на 1080p без записи объектов в буфер моделей и без паники. Измеримые критерии шага 6.

## Суть
Слой растительности хранит массив экземпляров в GPU-буфере. Compute-проход отсекает их по фрустуму и Hi-Z, выбирает LOD по расстоянию и пишет `DrawIndexedIndirect` по LOD. Основной проход, префасс нормалей, тени и скорость читают экземпляры из того же буфера. Плавное исчезновение по дальности через существующий blue-noise dither.

## Что уже есть (проверено чтением)
- Образцы GPU-отсечения с indirect: `cluster_gpu_cull.rs`, `virtual_geometry/gpu_cull.rs`, `shaders/vg_cull.wgsl`, `vg_dag_traverse.wgsl`; Hi-Z: `shaders/hiz_*.wgsl`, `virtual_geometry/hiz.rs` (**ПРОВЕРИТЬ** путь).
- Флаг устройства `INDIRECT_FIRST_INSTANCE` включён (исправление E1 из `FOREST_SCENE_FINDINGS.md`).
- Варианты материалов для проходов строятся в `matgraph_passes.rs` (`GraphPass::{Forward, VsmDepth, Velocity, …}`), ветер задаётся WPO-графом.
- Stochastic alpha: `shaders/material_surface.wgsl` (`tex.dither`).
- Процедурная расстановка: `crates/redox_scene/src/pcg.rs`; ячейки мира: `redox_scene/src/partition.rs`.

## Шаг 1 — данные слоя и compute-отсечение без отрисовки (M)
- Модуль `redox_render/src/foliage/{mod,layer,cull}.rs`, шейдер `shaders/foliage_cull.wgsl`.
- `FoliageLayer { lods: Vec<LodRef {mesh_slot, max_distance}>, material_slot, instances: Vec<FoliageInstance { pos: [f32;3], yaw: f32, scale: f32, seed: u32 }> }`; GPU-буфер экземпляров (storage), счётчики по LOD, буфер `DrawIndexedIndirect` по LOD, буфер видимых индексов.
- CPU-эталон `cull_reference(instances, camera, lods) -> Vec<(lod, index)>`.
- Тесты (GPU): `the_gpu_cull_matches_the_cpu_reference` (случайные 5 000 экземпляров; наборы видимых индексов по LOD совпадают), `instances_behind_the_camera_are_culled`, `lod_follows_the_distance_with_hysteresis`.

## Шаг 2 — отрисовка слоя в основном проходе (M; нужен шаг 1)
- Вершинный шейдер читает экземпляр по `instance_index` через буфер видимых индексов; материал тот же (`Material` из слота), двусторонний по MAT-08.
- Проход встроить в граф кадра рядом с основным (`frame_graph.rs`, **ПРОВЕРИТЬ** как объявляются проходы и зависимости чтения/записи; тест `every_declared_read_and_write_is_used_by_the_pass` должен остаться зелёным).
- Тесты (GPU): `a_layer_draws_the_same_picture_as_individual_objects` (100 экземпляров слоем и 100 `RenderObject`: кадры в допуске 3/255), `a_frame_without_layers_is_bitwise_unchanged`.

## Шаг 3 — тени, префасс нормалей, скорость (M; нужен шаг 2)
- Варианты слоя для VSM-страниц (`shadow/depth_pass.rs`, `vsm_depth_masked.wgsl`), нормального префасса (`pass/normal.rs`), скорости (`pass/velocity.rs`).
- Тени растительности ограничить по дальности (`shadow_distance` слоя; по умолчанию 40 м), чтобы не перерисовывать страницы VSM тысячами мелких кастеров.
- Тесты: `layer_instances_cast_shadows_inside_the_shadow_distance`, `layer_instances_beyond_the_shadow_distance_cast_none`, `a_moving_layer_writes_velocity` (ветер WPO).

## Шаг 4 — плавное исчезновение и ветер (S)
- Исчезновение по дальности: порог dither растёт с расстоянием, экземпляр не пропадает скачком; проверка по двум кадрам на разной дальности.
- Ветер: слой использует графовый материал с `PositionOffset` (MAT-03) — тест `a_wind_graph_moves_layer_instances`.

## Шаг 5 — данные расстановки (M; крейт `redox_scene`)
- `FoliageRule { model, density_per_m2, min_scale, max_scale, max_slope, height_range, mask }` в `pcg.rs` (расширение, не замена) и `scatter_layer(rule, cell, seed, height_fn) -> Vec<FoliageInstance>`; детерминированно по `(seed, ячейка)`.
- Хранение по ячейкам `WorldPartition` (**ПРОВЕРИТЬ** формат OFPA в `ofpa.rs`; если запись экземпляров слоя в OFPA не подходит, записать ограничение и хранить как отдельный бинарный блок ячейки).
- Тесты: `scatter_is_deterministic`, `scatter_respects_slope_and_height`, `adjacent_cells_do_not_overlap_instances`.

## Шаг 6 — замер и golden (S)
- Сцена `examples/forest` с 200 000 лезвий слоем: кадр 1080p, замер GPU-времени проходов и CPU-времени кадра, память. Таблица в `docs/BENCHMARKS.md`.
- Приёмка: ≤ 16,6 мс на кадр на машине владельца **или** 📌 с измеренным числом и главным участком времени; golden-кадр леса.

## Границы
- **Читать:** `cluster_gpu_cull.rs`, `virtual_geometry/{gpu_cull,hiz}.rs`, `shaders/{vg_cull,hiz_*}.wgsl`, `frame_graph.rs`, `renderer/{frame,passes}.rs`, `matgraph_passes.rs`, `redox_scene/src/{pcg,partition,ofpa}.rs`.
- **Создать:** `src/foliage/`, `shaders/foliage_*.wgsl`, `tests/foliage_gpu.rs`.
- **Менять:** `lib.rs` (модуль), `frame_graph.rs` (проход), `renderer/frame.rs`, шейдеры вариантов, `redox_scene/src/pcg.rs`, `examples/forest`.
- **Не трогать:** VG, GI, туман.

## Проверка
`cargo fmt`, `cargo check -p redox_render -p redox_scene --all-targets`; свои тесты по одному. Clippy, полный прогон и замер — координатор.

## Для исполнителя
См. `../FOREST_WAVE.md` §4. Отчёт: `../notes/FOL-03-report.md`. Один шаг за один заход.

## Закрытие
✅ / ⛔ / 📌 по шагам. Если шаги 2 или 3 остаются красными после доработки, координатор берёт их сам; если не получилось, исход 📌 с измеренным числом.

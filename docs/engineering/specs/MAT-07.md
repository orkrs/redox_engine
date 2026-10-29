# MAT-07 — Материалы, E7: интеграция — VG resolve, тени, скорость, варианты шейдеров

- **Статус:** не начат.

- **Размер:** 4 шага (M, M, S, S) ≈ 5–6 дней. **Крейт:** `redox_render`. **Зависит от:** MAT-02, MAT-03 (WPO), MAT-04, MAT-05, MAT-06 (шаги спеки включаются по мере готовности: каждый шаг помечен нужной зависимостью). **Блокирует:** ничего.
- **Модель:** haiku с проверкой координатора после каждого шага.
- Объём — «минимум» плана E7. **Не входит:** классификация тайлов по сложности материала.

## Суть
Граф/слои/SSS/прозрачность работают в основном проходе. Остальные проходы, которые видят материал, должны давать согласованный результат: VG resolve, теневые проходы (с WPO и маской), скорость (для TAA и motion blur), нормальный префасс (GTAO/SSR), захват для GI. Число вариантов шейдера под контролем.

## Что уже есть (проверено чтением)
- `vg_resolve.wgsl` (887 строк): отдельный расчёт материала (после MAT-01 — общая `material_surface`); `vg_shadow.wgsl`, `vg_vis.wgsl`.
- Проходы, читающие материал/вершины: `normal.wgsl` (нормаль+глубина), `shadow.wgsl` и `vsm_depth.wgsl` (глубина теней), `velocity.wgsl` и `object_velocity.wgsl` (скорость), основной `pbr.wgsl`, прозрачный (тот же `pbr.wgsl`).
- Кэш конвейеров: `pipeline_cache.rs`; ключи вариантов графа: MAT-02 шаг 5.

## Шаг 1 — WPO во всех геометрических проходах (M; нужен MAT-03 шаг 4)
Все проходы, трансформирующие вершины, должны применять `material_vertex_offset` для материалов с `Features::vertex_anim`:
`normal.wgsl`, `shadow.wgsl`, `vsm_depth.wgsl`, `object_velocity.wgsl`/`velocity.wgsl` (скорость WPO = смещение текущего кадра − смещение предыдущего: вычисляется в шейдере с `time` и `prev_time` из uniform; если `time` не передаётся предыдущего кадра — **ПРОВЕРИТЬ** `camera_uniform`/`shader_debug`, добавить `prev_time`).
Генерация вариантов: ключ `(граф, Pass::{Main, Normal, Shadow, Velocity}, Features)`; `generate` (MAT-02) уже умеет `Pass`; добавить `Normal` и `Velocity` в `Pass` и в тесты `every_pass_validates_with_naga`.
Тесты (GPU): `wpo_shadows_follow_the_displaced_geometry` (плоскость с WPO вверх на 1 м: тень на земле смещается — центр тёмного пятна сдвинут по ожиданию ± 3 пикселя); `wpo_velocity_matches_the_displacement` (объект с WPO по `sin(time)`: текстура скорости в его пикселях ненулевая и направлена по смещению; `taa_velocity_tex`, чтение как `tests/object_velocity.rs`); `wpo_normals_prepass_matches_the_main_depth` (GTAO не создаёт полос: разница глубин префасса и основного ≤ 1e-3 на пикселях объекта).

## Шаг 2 — VG resolve и материалы графа (M; нужен MAT-02)
- Материал в VG: «каждый meshlet ассета делит один `material_index`» (`vg_vis.wgsl`: восстановление через `meshlets[instances[i].meshlet_offset]`). Для графовых материалов resolve-проход должен вызывать **сгенерированную** `material_surface_graph`: вариант `vg_resolve` на материал → N проходов по числу графовых материалов в кадре (бининг по материалу — по маске `material_index` в visibility; **упрощение v1:** отдельный полноэкранный проход на материал с отсечением пикселей по `material_index` (scissor не нужен — `discard` по несовпадению), число материалов VG в кадре ограничить `MAX_VG_GRAPH_MATERIALS = 8` с предупреждением в лог при превышении (лишние рисуются запасным материалом).
- Тесты (GPU): `a_graph_material_on_a_vg_mesh_matches_the_forward_path` (тот же меш и материал через forward и VG: кадры в допуске 3/255 на пикселях меша); `two_graph_materials_in_one_vg_frame` (оба цвета видны); `more_than_eight_materials_fall_back_without_crashing`; `vg_without_graph_materials_is_bitwise_unchanged` (golden/существующие VG-тесты).

## Шаг 3 — маска в VG-тенях и скорость (S; нужен MAT-06 шаг 3)
`vg_shadow.wgsl`: для alpha-test материалов ассета — отбрасывание по альфе (одна текстура/параметр на ассет: источником служит `material_opacity` из MAT-06; для VG-ассетов с alpha-test вывод «VG не подходит для alpha-test листвы» из `PLAN_GEOMETRY.md` §1.6 остаётся в силе — **в этой спеке только проверка, что при попадании такого ассета в VG-путь он не ломается**: логируется предупреждение при регистрации (`VGSystem::register_asset(…)` для материала с alpha-test флагом) и ассет рисуется классическим путём — реализовать проверку на уровне `register_*` и тест `an_alpha_tested_asset_is_not_registered_as_vg`).
Скорость VG: **ПРОВЕРИТЬ** используется ли `object_velocity.wgsl` для VG-экземпляров; если нет — вне объёма (записать в отчёт).

## Шаг 4 — контроль числа вариантов (S)
- `MaterialShaderCache::stats() -> CacheStats { modules, hits, misses }`; предупреждение в лог, когда модулей > `MAX_VARIANTS_WARN = 256`.
- Тест: `the_variant_count_stays_bounded_for_one_graph_across_passes` (один граф × 5 проходов × 3 набора features ≤ 15 модулей); `identical_graphs_with_different_names_share_modules` (хеш по содержимому IR, а не по имени): записать результат.
- Документ `docs/` (раздел «Материалы»): таблица «проход × что применяется (WPO, маска, граф)».
- Абзац в `PLAN_MATERIALS.md` §6–7, `PLAN_EXECUTION.md` E7 → ✅, `CHANGELOG.md`.

## Границы
- **Читать:** MAT-02…MAT-06, `shaders/{vg_resolve,vg_shadow,vg_vis,normal,shadow,vsm_depth,velocity,object_velocity}.wgsl`, `virtual_geometry/runtime.rs` (`register_asset`), `matgraph_cache.rs`, `tests/object_velocity.rs`.
- **Создать:** `tests/material_integration.rs`.
- **Менять:** перечисленные шейдеры, `redox_matgraph/src/codegen.rs` (`Pass`), `matgraph_cache.rs`, `virtual_geometry/runtime.rs` (проверка alpha-test).
- **Не трогать:** SSS/SSR/туман.

## Проверка
`cargo fmt`, `cargo clippy -p redox_matgraph -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_matgraph`, `cargo test -p redox_render` (весь крейт), `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаги 1–4 независимы по зависимостям: пропусти шаг, если его зависимость не в `main`, и отметь это в отчёте.

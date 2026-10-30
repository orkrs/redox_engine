# MAT-03 — Материалы, E3: набор узлов v1, custom node, смещение вершин (WPO)

- **Статус:** шаги 1–3 написаны в `redox_matgraph` (узлы текстур, математика и шум, custom node с проверкой naga); `cargo check -p redox_matgraph --all-targets` чистый; тесты, clippy и GPU-сверка не запускались (координатор). Шаг 4 (WPO) не делался: он правит `redox_render`, другой агент. Не сделано внутри шагов: `HeightBlend` и `NormalFromHeight` (нужны производные `dpdx/dpdy`, которых у CPU-интерпретатора нет; решение по семантике не принято); `redox_render/shaders/matgraph_noise.wgsl` не создан (WGSL шума генерируется из крейта, `noise::WGSL`); `eval` для custom возвращает нулевое значение, а `try_eval` — `None` (вместо `eval -> None`, чтобы не менять сигнатуру для `redox_render`).

- **Размер:** 4 шага (S, M, S, M) ≈ 5–6 дней. **Крейты:** `redox_matgraph`, `redox_render`. **Зависит от:** MAT-02 (шаги 1–5). **Блокирует:** MAT-07.
- **Модель:** haiku. Каждый узел добавляется **по единому шаблону из трёх мест**: тип/порты (`graph.rs`), выражение WGSL (`emit.rs`), интерпретатор (`eval`); тест «каждый узел имеет генератор» из MAT-02 шаг 3 обязан остаться зелёным.

## Суть
Чтобы описать материалы Courier и ландшафта графами, нужны текстуры, больше математики, шум, triplanar, свой фрагмент WGSL и смещение вершин (ветер, волны).

## Шаг 1 — текстурные узлы (S–M)
Новые `NodeKind`: `Texture2d { slot: u8 }` (входы: `uv`; выходы: `rgba`), `TextureSample` (рейтинг как у `Texture2d` + семплер), `NormalMap { slot }` (распаковка `xy*2-1`, восстановление `z`, TBN из `SurfaceInput`), `Triplanar { slot, scale }` (три выборки по осям мировой позиции, веса `pow(abs(n), 4)` нормированные).
Привязка текстур: графовые слоты `slot` → реестр текстур (`registry`: **ПРОВЕРИТЬ** как сейчас выбирается `texture_index` материала: `material.rs:13–17`, массив текстур/bindless? `grep -n "texture_index\|binding_array" crates/redox_render/shaders/pbr.wgsl crates/redox_render/src/registry.rs`). Текстурные слоты = параметры экземпляра типа `Texture2d` (индекс в массиве), не константы графа.
Тесты: `a_texture_node_validates_and_generates` (naga OK); `a_triplanar_weights_sum_to_one` (интерпретатор: вход нормаль (0.6,0.8,0) → веса суммируются в 1 ± 1e-6); `a_normal_map_flat_texel_gives_the_surface_normal` ((0.5,0.5,1) → нормаль не меняется); GPU: `a_graph_with_a_texture_matches_the_legacy_albedo_texture` (граф «Texture2d → base_color» vs `Material::texture(i)` → пиксели в допуске 2/255).

## Шаг 2 — математика и шум (M)
Новые узлы: `Abs, Floor, Fract, Min, Max, Saturate, Step, Smoothstep, Sin, Cos, Sqrt, Fresnel { power }, Remap { in_min, in_max, out_min, out_max }, Split/Merge (swizzle: ComponentMask), If { mask }` (`select`), `ValueNoise, GradientNoise, Worley` (2D/3D по `Vec2/Vec3`; **функции шума — в WGSL-прелюдии `matgraph_noise.wgsl`** и зеркально в Rust для эталона: хеш — целочисленный PCG без `sin`), `HeightBlend`, `NormalFromHeight` (по `dpdx/dpdy`).
Тесты:
```rust
#[test] fn rust_and_wgsl_noise_agree_on_a_grid_of_points() { /* GPU compute: 64 точек, допуск 1e-4 против Rust-реализации */ }
#[test] fn value_noise_stays_in_zero_one() { /* 10000 точек */ }
#[test] fn noise_is_deterministic_and_continuous() { /* значения в соседних точках (eps 1e-3) отличаются < 0.05 */ }
#[test] fn smoothstep_clamps_outside_the_range() { /* ... */ }
#[test] fn fresnel_is_zero_facing_and_one_at_grazing() { /* ... */ }
#[test] fn if_selects_by_the_mask() { /* mask 0/1 */ }
```
Для каждого нового узла — запись в таблице типов (`input_types/output_type`), проверка `every_node_kind_has_a_generator` (расширяется автоматически).

## Шаг 3 — custom node (S)
```rust
NodeKind::Custom { name: String, inputs: Vec<(String, Ty)>, output: Ty, body: String }
```
Пользовательский фрагмент WGSL: тело функции `fn custom_{id}({inputs}) -> {output} { body }`. Проверка: сборка функции в минимальном модуле и разбор `naga` → ошибка с **номером строки внутри `body`** (вычесть смещение заголовка) и `node id`. Лимиты: длина `body` ≤ 4 КиБ; запрещены `@group/@binding`, `var<`-глобалы, `enable` (проверка по тексту токенами); число текстурных выборок во всём графе ≤ 16 (предупреждение `GraphWarning`, не ошибка).
Тесты: `a_valid_custom_node_generates_and_validates`; `a_syntax_error_reports_the_line_inside_the_body` (ошибка в строке 2 тела → «строка 2»); `a_custom_node_cannot_declare_bindings`; `an_oversized_body_is_rejected`; `a_custom_node_in_a_graph_is_evaluated_on_the_gpu` (тело `return a * 2.0;` — совпадает с ожидаемым значением).
Интерпретатор Rust такой узел не вычисляет: `eval` возвращает `None` — тесты сверки пропускают графы с `Custom`.

## Шаг 4 — смещение позиции вершин (WPO) (M)
Новый выход графа: `PositionOffset (Vec3)` в мировом пространстве; генерация второй функции `fn material_vertex_offset(in: VertexInput, params) -> vec3<f32>` (входы: `world_pos`, `normal`, `uv`, `time`, `vertex_color`). Допустимые узлы вершинной части: `WorldPos, Normal, Uv, VertexColor, Time`, математика, шум; текстурные выборки в вершинной части — только `textureSampleLevel` (в v1 запрещены; проверка графа).
Подключение: в `vs_main` (`pbr.wgsl:175`) — `world_pos += material_vertex_offset(...)` для материалов с WPO (вариант шейдера `Features::vertex_anim`); **те же вершины** обязаны сместиться в проходе глубины/теней/скорости → в MAT-07 (здесь только основной проход + проход глубины нормалей: `shaders/normal.wgsl`, **ПРОВЕРИТЬ** `NormalPass` — вариант для WPO-материалов).
Ограничение v1: границы объекта для отсечения не расширяются → `RenderObject::bounds` дополняется `Material::wpo_max_offset` (число метров, задаётся графом константой на выходе `PositionOffsetMaxLength`); тест `bounds_grow_by_the_declared_wpo_offset`.
Тесты: `a_wpo_graph_moves_the_vertices` (GPU: куб с `sin(time)`-смещением по Y: на двух значениях `time` кадры различаются в строках верхней грани); `a_material_without_wpo_is_bitwise_unchanged`; `wpo_in_a_fragment_only_node_set_is_rejected` (граф с `dpdx` в вершинной части → ошибка с id).

## Границы
- **Читать:** MAT-02, `material.rs`, `shaders/pbr.wgsl` (`vs_main`), `shaders/normal.wgsl`, `registry.rs`, `PLAN_MATERIALS.md` §1.3.
- **Создать:** `shaders/matgraph_noise.wgsl`, `redox_matgraph/src/{noise,custom}.rs`, `redox_render/tests/matgraph_nodes.rs`.
- **Менять:** `redox_matgraph/src/{graph,emit,eval}.rs`, `redox_render/src/matgraph_cache.rs`, `pbr.wgsl` (вершинный вызов), `systems.rs` (границы).
- **Не трогать:** тени/VG (MAT-07).

## Проверка
`cargo fmt`, `cargo clippy -p redox_matgraph -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_matgraph`, `cargo test -p redox_render --test matgraph_gpu --test matgraph_nodes --test golden_frames`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. MAT-02 в `main`. Новые узлы — только из списка выше; «редкие узлы» в план не входят.

## Закрытие (финальная волна)

Исход обязателен: ✅ / ⛔ / 📌 (`../FINAL_WAVE.md` §1). Агенту разрешён `cargo check -p <крейт> --all-targets` в своём worktree; тесты, clippy и замеры — у координатора. Шаг, у которого приёмочный тест после доработки красный, не остаётся «частично»: агент сообщает измеренное число, исход назначает координатор.

- **Закрытие:** 4 шага по порядку после MAT-02; custom node — чистые функции WGSL без доступа к привязкам; WPO согласован с деформацией RND-08.
- **Приёмка:** тест на каждый узел v1 сверяется с интерпретатором `eval`; WPO-тест: тень совпадает с геометрией ±1/255.
- **Исход (2026-10-30): шаги 1–3 ✅ в `redox_matgraph`** (текстурные узлы, математика и шум, custom node), тесты зелёные. Остаётся: HeightBlend и NormalFromHeight (нужны производные dpdx/dpdy, у CPU-интерпретатора их нет — решить семантику), шаг 4 (WPO) и GPU-тесты `a_graph_with_a_texture_matches_the_legacy_albedo_texture`, `rust_and_wgsl_noise_agree`, `a_custom_node_in_a_graph_is_evaluated_on_the_gpu` — после MAT-02 (агент R5).
- **Не выполнено в финальной волне:** шаг 4 (WPO), узлы HeightBlend/NormalFromHeight, GPU-тесты рендера (`a_graph_with_a_texture_matches_the_legacy_albedo_texture`, `rust_and_wgsl_noise_agree`, `a_custom_node_in_a_graph_is_evaluated_on_the_gpu`).
- **Обновление (2026-10-30):** шаг 4 ✅ — узел `PositionOffset { max_offset }`, функция `material_vertex_offset_graph`, проверка «фрагментные узлы не питают смещение», узлы `HeightBlend` и `NormalFromHeight`, расширение границ отсечения на `max_offset`. Тесты `mat07_gpu`.
- **Исход шага 1 рендерера (текстуры в рендерере): ✅.** Проверка слота `GRAPH_TEXTURE_SLOTS = 4` в `redox_matgraph`; группа параметров графа содержит `tex0..tex3` (binding 2..5, FRAGMENT) и сэмплеры `samp_default/repeat/clamp` (binding 6..8); незаполненные слоты используют белую текстуру. Привязка: `Renderer::set_material_graph_with_textures(material, graph, params, texture_indices)`; модули forward, depth, shadow и VSM объявляют привязки. Тесты: `mat03_tex_gpu` (3 теста: граф с текстурой совпадает с `Material::textured` в допуске 4/255; пустой список текстур не меняет кадр; NormalMap и Triplanar рисуются), `matgraph_gpu`, `mat07_gpu`, `golden_frames`, `--lib`.
- **Известное ограничение:** горячая перезагрузка (`MaterialGraphHotReload::update`) и `set_material_instance` передают пустой список текстур, так что после перезагрузки текстурные слоты белые; нужно передавать индексы текстур тем же путём.
- **Известный красный тест (не из этого шага):** `matgraph_draw::two_instances_share_one_shader_with_different_params` ожидает 3 модуля, а `set_material` строит 4 (forward, depth, shadow, VSM); расхождение было до этого шага.

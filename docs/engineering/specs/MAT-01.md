# MAT-01 — Материалы, E1: единая поверхностная функция `material_surface` (предпосылка графа материалов)

- **Статус:** выполнено 2026-10-29, кадры побитно прежние (VG resolve пока на своём пути).

- **Размер:** M (3 шага). **Крейт:** `redox_render`. **Зависит от:** ничего. **Блокирует:** MAT-02…MAT-07 (без этой функции граф нечем подключать).
- **Модель:** haiku. **Картинка не меняется**: `golden_frames` и все кадровые тесты — побитно; эталоны не обновлять.
- Замечание координатора: E1 в плане (`PLAN_EXECUTION.md:216`) помечен ⬜ и стоит *перед* E2. Запрос «E2–E7» без E1 не выполним — поэтому E1 оформлен отдельной спекой.

## Суть
Параметры материала сейчас разбросаны по телу `fs_main` (`shaders/pbr.wgsl:690–1000`) и дублируются в `shaders/vg_resolve.wgsl` (887 строк). Прежде чем строить кодогенерацию, нужна **одна** функция, которую вызывают все проходы, и одна структура результата.
`grep -rn material_surface crates/redox_render` ничего не находит — функции нет.

## Что уже есть (проверено чтением)
- `Material` (CPU, `material.rs:9–30`): `base_color, texture_index, normal_texture_index, mr_texture_index, metallic, roughness, emissive, layers: Option<[MaterialLayer;2]>, sheen`; `MaterialUniform` (GPU, `material.rs:38–60`) с `flags` (`HAS_SUBSTRATE = 1<<3`, есть флаги текстур и alpha-test `16u`).
- В `pbr.wgsl`: структура `Material` (стр. 25–36), чтение текстур albedo/normal/MR, расчёт `alpha`, dither-discard (`~696–707`), `eval_brdf_lobe`, `eval_substrate_lit`, `eval_sheen_lobe` (586–663), IBL-слои (929+).
- `vg_resolve.wgsl` содержит свою копию расчёта материала для VG (**ПРОВЕРИТЬ**: `grep -n "metallic\|roughness\|layer0" shaders/vg_resolve.wgsl`).
- Общие куски WGSL подключаются строковой склейкой: `shader::resolve_source`, `shader_with_vertex_decode` (`shader/mod.rs:273–298`) — прецедент для общего модуля.

## Шаг 1 — структура поверхности и функция в `pbr.wgsl` (M)
```wgsl
struct SurfaceInput {
    world_pos: vec3<f32>, world_normal: vec3<f32>, uv: vec2<f32>, view_dir: vec3<f32>,
    vertex_color: vec4<f32>, instance_color: vec4<f32>, frag_coord: vec4<f32>,
};
struct SurfaceData {
    base_color: vec3<f32>, opacity: f32, normal: vec3<f32>, metallic: f32, roughness: f32, emissive: vec3<f32>,
    // Substrate-лайт (как сейчас): до двух дополнительных слоёв и sheen
    layer0: vec4<f32>, layer0_params: vec4<f32>, layer1: vec4<f32>, layer1_params: vec4<f32>, sheen: f32,
};
fn material_surface(in: SurfaceInput) -> SurfaceData
```
Тело — **перенос** кода чтения текстур/параметров из `fs_main` без изменения формул. `fs_main` вызывает `material_surface`, остальной код (освещение, тени, IBL) читает поля `SurfaceData`.
Порядок операций с плавающей запятой сохранить (иначе «побитно» не выйдет): выражения переносятся дословно, не упрощаются.
Тесты: все существующие кадровые тесты зелёные **без правок** (`golden_frames`, `object_velocity`, `post_fx`, `instancing`, `material_textures`); `cargo test -p redox_render` целиком. Новый тест `tests/material_surface.rs`: `the_surface_function_returns_the_material_parameters` — рендер куба с `Material { metallic:0.7, roughness:0.3, ..}` и чтение... (GPU-чтение значений функции без отладочного вывода сложно) — вместо него тест **структуры**: `naga`-разбор `PBR_WGSL` и проверка, что есть функция `material_surface` с одним параметром и возвращаемым типом `SurfaceData` (парсер `naga::front::wgsl` уже зависимость, `Cargo.toml:61`).

## Шаг 2 — общий WGSL-модуль и VG resolve (M)
- Вынести `SurfaceInput`, `SurfaceData`, `Material`, `material_surface` в `shaders/material_surface.wgsl`, склеиваемый перед `pbr.wgsl` и `vg_resolve.wgsl` (механизм — по образцу `shader_with_vertex_decode`; **ПРОВЕРИТЬ** привязки: у PBR и VG-resolve разные `@group/@binding` для текстур — модуль не должен объявлять привязки, он принимает значения текстур параметрами или объявляет функции-заглушки `sample_albedo(uv)`, реализуемые каждым проходом).
  Решение по умолчанию: **модуль содержит только типы и чистые функции**, а чтение текстур — `fn material_sample_textures(in) -> TextureSamples` в каждом шейдере; `material_surface(in, tex)` общая.
- `vg_resolve.wgsl`: заменить собственный расчёт на вызов общей функции.
- Тесты: VG-кадровые тесты (`grep -rln "vg\|VG" tests`) зелёные, эталоны не меняются; `cargo test -p redox_render --test reload_shaders` (горячая перезагрузка видит новый файл: добавить его в список `ShaderWatcher`, **ПРОВЕРИТЬ** `shader/watch.rs`).

## Шаг 3 — экземпляры материала (S)
Цель: параметры, помеченные «параметр», лежат в uniform экземпляра, а не в шейдере (нужно графу).
- `MaterialInstanceId` = индекс слота `Material` (уже есть: `material_index`, `registry.legacy_material_bind_group`); оформить это словами и типом: `pub struct MaterialInstance { pub graph: Option<MaterialGraphId>, pub material: Material }` — **поле `graph` пока всегда `None`** (заготовка для MAT-02), поведение не меняется.
- Тест: `a_material_instance_without_a_graph_renders_as_before` (golden).
Абзац в `PLAN_MATERIALS.md` §1.1 («M1 выполнено»), `PLAN_EXECUTION.md` E1 → ✅, `CHANGELOG.md`.

## Границы
- **Читать:** `shaders/pbr.wgsl` (целиком), `shaders/vg_resolve.wgsl`, `material.rs`, `registry.rs`, `shader/{mod,watch}.rs`, `tests/reload_shaders.rs`, `tests/material_textures.rs`.
- **Создать:** `shaders/material_surface.wgsl` (шаг 2), `tests/material_surface.rs`.
- **Менять:** `pbr.wgsl`, `vg_resolve.wgsl`, `shader/mod.rs`, `material.rs` (шаг 3).
- **Не трогать:** остальные шейдеры, эталоны.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render` (весь крейт), `python scripts/check_file_sizes.py` (`pbr.wgsl` 1227 строк — **сократится** после выноса; проверить, что скрипт учитывает `.wgsl`).

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Если «побитно» недостижимо без изменения формул — остановись и опиши разницу (метрика `max_channel_diff`), не подгоняй эталоны.

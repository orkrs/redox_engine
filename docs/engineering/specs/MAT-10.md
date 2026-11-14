# MAT-10 — `MaterialInstance.graph` доходит до кадра и до превью редактора

- **Статус:** не начат (аудит кода 2026-11-15).
- **Размер:** 3 шага (M, M, S) ≈ 5 дней. **Крейты:** `redox_render`, `redox_core`, `redox_editor`. **Зависит от:** MAT-02 (`set_material_graph`). **Блокирует:** —.
- **Модель:** haiku. Превью — с проверкой координатора на GPU.
- Граф, сохранённый в материале, сам попадает в слот рендера. **Не входит:** новые узлы графа, coat в `SurfaceData` (MAT-04), UV на VG-resolve (MAT-07).

## Суть
Живой путь — `Renderer::set_material_graph`. Его вызывают тесты и `redox_core/src/material_graph.rs`, когда слот уже связали вручную. `MaterialInstance.graph` в `material.rs` — это `Option<MaterialGraphId>`, по комментарию «граф ещё не загружен», и кадр это поле не читает. KNOWN_LIMITS MAT-02 пишет, что VSM рисует слот без графа: в коде уже есть `GraphPass::VsmDepth` и привязка в `renderer/contexts.rs`. Оставшаяся дыра — потребитель `MaterialInstance`, плюс превью редактора, которое EDT-B5 не повесил на `RenderHost` (`preview_revision` не двигает сферу в viewport).

## Что уже есть
- Компиляция и hot-reload графа, редактор `.matgraph` (документы и холст в `redox_editor_assets` / `redox_editor_ui`).
- `set_material_graph` при ошибке оставляет предыдущий граф.
- VSM-вариант графа создаётся в `matgraph_cache.rs`.

## Шаг 1 — слот читает инстанс (M)
- При сборке кадра инстанс с `graph: Some(id)` резолвится в тот же вызов, что `set_material_graph` (загрузка документа по id, параметры инстанса). `None` оставляет обычный `Material` и снимает граф слота, если он был.
- Неизвестный id — ошибка в лог один раз, слот остаётся на прежнем графе или на обычном материале, кадра не роняет.
Тесты: `an_instance_without_a_graph_uses_the_plain_material`; `an_instance_with_a_graph_binds_that_graph`; `a_missing_graph_id_keeps_the_previous_binding`.

## Шаг 2 — превью редактора (M)
- Сфера (или куб) в превью материала рисуется через `RenderHost` тем же `set_material_graph`, что игра. Смена `preview_revision` пересобирает вариант.
- Ошибка компиляции не сохраняет файл (правило EDT-B5) и не меняет последнюю успешную сферу.
Тест GPU: `the_preview_sphere_shows_the_graph_colour` (граф «константа зелёного» → зелёный пиксель на сфере). `a_failed_compile_keeps_the_previous_preview`.

## Шаг 3 — VSM и документация (S)
- Тест GPU: объект с графом, который сдвигает вершину или красит альбедо, даёт другой VSM-отпечаток, чем тот же слот без графа. Если `GraphPass::VsmDepth` уже это делает — тест фиксирует поведение, шейдер не переписывать.
- Строку KNOWN_LIMITS MAT-02 сузить до того, что тест ещё не покрывает. Если оба теста зелёные — строку снять. `CHANGELOG.md`.

## Границы
- **Читать:** MAT-02, EDT-B5, `material.rs`, `matgraph_cache.rs`, `renderer/contexts.rs`, `redox_core/src/material_graph.rs`.
- **Менять:** место, где кадр выбирает пайплайн слота; превью в редакторе.
- **Не трогать:** генератор WGSL, лимит 8 графовых слотов на VG (MAT-07).

## Проверка
`cargo fmt`, `cargo clippy -p redox_render -p redox_core -p redox_editor --all-targets -- -D warnings`, `cargo test -p redox_render --test matgraph_draw`, GPU-превью у координатора.

## Для исполнителя
`git merge --ff-only main`. Не заводить второй компилятор в редакторе. Если `MaterialGraphId` ни на что не указывает в ассетах — шаг 1 сначала заводит таблицу id → граф в том крейте, где уже лежит документ `.matgraph`, и только потом читает её из кадра.

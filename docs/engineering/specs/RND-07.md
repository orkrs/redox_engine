# RND-07 — GPU-сцена и indirect-отрисовка (A5): ворота по замеру, затем два шага

- **Статус:** закрыто по замеру 2026-10-29: CPU кадра 1,39–1,62 мс при 1–400 группах (3000 объектов, release) — A5 не нужна.

- **Размер:** S (ворота) + M (шаг 1) + L (шаг 2). **Крейт:** `redox_render`. **Зависит от:** RND-04 (готово). **Блокирует:** ничего.
- **Модель:** haiku для ворот и шага 1; шаг 2 — только после письменного «да» владельца по результату ворот.
- **Файлы одному исполнителю:** `renderer/draw.rs`, `renderer/frame.rs`, `renderer/batching.rs`, `renderer/mod.rs`, `renderer/init.rs`.

## Суть
В плане A5 — «GPU-сцена»: instance-буфер на GPU, отсечение на compute, indirect-draw. Нужна, только если CPU кадра > 4 мс. Замер RND-04 дал **1,45 мс на 3000 кубов**
(подготовка 0,03; запись 1,03; отправка 0,39), но это **лучший случай**: все кубы — один меш и один материал, то есть **один** вызов отрисовки на весь список.
Реальная сцена — десятки мешей и материалов, и стоимость записи растёт с числом групп, а не объектов. Поэтому сначала честные ворота.

## Что уже есть (проверено чтением)
- Отрисовка непрозрачных (`renderer/draw.rs:39–83`): объекты уже сортируются по (меш, материал) (`batching::draw_order`, `batching.rs:17`), соседние одинаковые склеиваются в **пробеги** (`batching::runs`, `batching.rs:49`),
  на пробег — `set_bind_group(2, material)`, `set_vertex_buffer`, `set_index_buffer`, один `draw_indexed(0..index_count, 0, run)`. Счётчик `forward_draw_calls`.
- Матрицы моделей уже лежат в **storage-буфере** (`model_bind_group`, группа 3); индекс экземпляра = индекс объекта (`draw.rs:105` упоминает «instance indices stay GLOBAL»). То есть «instance-буфер на GPU» уже есть; данные загружаются каждый кадр.
- Видимость считается на CPU: `compute_main_visibility` (экспорт в `renderer/mod.rs:40`), плюс GPU-отсечение кластеров света (`cluster_gpu_cull.rs`) и **виртуальная геометрия с indirect** (`virtual_geometry/gpu_cull.rs`, `indirect.rs: IndirectDrawBuffer`, `draw_indexed_indirect`) — готовая основа для indirect.
- Замер: `FrameCpuTimings { prepare_ms, record_ms, submit_ms }` (`renderer/frame.rs:10`, `Renderer::last_frame_cpu_timings`), тест-образец `tests/frame_timings.rs` (медиана кадров, `median_timings`).

## Шаг 0 — ворота: замер на разнородной сцене (S, 1 час)
Добавить в `tests/frame_timings.rs` тест-замер (не падает по времени, печатает таблицу при `--nocapture`, как существующий):
```rust
/// Objects spread over `groups` distinct (mesh, material) pairs, interleaved so that sorting has work to do.
fn mixed_objects(&self, count: usize, groups: usize) -> Vec<RenderObject>;   // meshes: create_cube/create_sphere/... uploaded once; materials: Material::solid with different colours
#[test] fn cpu_time_grows_with_groups_not_objects() {
    // for groups in [1, 20, 100, 400] { median_timings(3000, 9) } -> eprintln!("groups {g}: {t:?}")
    // assert!(t.total_ms() < 1000.0) only; the figures go to the report.
}
```
Меши — минимум 5 разных примитивов (`redox_render::mesh::primitive`, **ПРОВЕРИТЬ** список: `create_cube`, `create_sphere`, …), материалы — до 80 штук (`add_material`). Для 400 групп комбинируются меш×материал.
Запуск: `cargo test -p redox_render --release --test frame_timings -- --nocapture`.
**Решение по воротам (записать в `BACKLOG.md` и `PROGRESS.md`):**
- Худший вариант (400 групп) `total_ms ≤ 4` → A5 **закрыта как не нужная**, спека получает статус «закрыто по замеру». Шаги 1–2 не делать.
- `4 < total_ms ≤ 8` → шаг 1.
- `> 8` → шаг 1, затем вопрос владельцу про шаг 2.
Замер приводить на dev-машине владельца (RTX 4060 Ti, release); число CI-машины не годится.

## Шаг 1 — дешёвое снижение стоимости записи (M)
Цель: меньше команд на группу, без смены архитектуры. Делать только то, что профиль покажет главным (посмотреть `record_ms` по группам из шага 0):
1. Не искать bind group материала через `registry.legacy_material_bind_group` на каждый пробег — держать вектор `material_bind_groups: Vec<&BindGroup>` по индексу материала, заполняемый раз в кадр (или при изменении материалов). Меньше хеш-обращений.
2. Не вызывать `set_vertex_buffer`/`set_index_buffer`, если меш не изменился относительно предыдущего пробега (сортировка по (меш, материал) гарантирует длинные серии одного меша).
3. Не вызывать `set_bind_group(2, …)`, если материал совпал с предыдущим.
4. Кэш порядка отрисовки между кадрами: если список объектов и видимость не изменились (хеш `mesh_index/material_index/visible` по кадрам), переиспользовать `draw_order` (`batching.rs:17`). Для динамичных сцен кэш бесполезен — не усложнять.
Каждый пункт — отдельный коммит с повторным замером из шага 0 (таблица «до/после» в сообщении коммита). Картинка обязана совпасть побитно (`golden_frames`).
Тесты: логика пунктов 2–3 — чистая функция `fn state_changes(runs: &[(mesh, material)]) -> (mesh_sets, material_sets)` с юнит-тестами (серия одного меша → 1 смена меша; чередование → N); GPU-тест — `forward_draw_calls` не меняется.

## Шаг 2 — GPU-управляемая отрисовка (L; только по «да» владельца)
**Оговорка из аудита.** Indirect и compute-отсечение убирают CPU-видимость и сортировку (`prepare_ms`, у нас 0,03 мс — мизер), но **не убирают вызовы записи на группу** (`record_ms`, основная часть),
пока материалы привязаны по одной bind group на группу. Выигрыш по `record_ms` даст только безбиндинговый (bindless/массив текстур) путь материалов, а это отдельная большая задача (граф материалов, E2). Поэтому шаг 2 — это:
1. `GpuSceneBuffers`: пулы `instances` (матрица, id материала, AABB) и `draw_args` (`DrawIndexedIndirect` на группу) — расширение существующего `IndirectDrawBuffer` (`virtual_geometry/indirect.rs`).
2. Compute-проход `scene_cull.wgsl`: по AABB и пирамиде видимости (`FrameUniform`/`camera_uniform`, **ПРОВЕРИТЬ** где лежат 6 плоскостей) пишет `instance_count` групп и компактный список индексов видимых экземпляров.
   Шаблон — `shaders/vg_cull.wgsl` и `virtual_geometry/gpu_cull.rs`.
3. Отрисовка: на группу `set_bind_group(2, material)` + `draw_indexed_indirect(args, group*20)`. Если фича `MULTI_DRAW_INDIRECT` доступна (**ПРОВЕРИТЬ** `device.features()`), группы с одним материалом — одним `multi_draw_indexed_indirect`.
4. Флаг `Renderer::set_gpu_scene(bool)`, по умолчанию выкл; выкл = нынешний путь, картинка побитно прежняя.
5. Тест: одна сцена, оба пути; `golden`-сравнение с допуском 1/255; число видимых экземпляров из readback совпадает с CPU-видимостью.
Решение владельца перед стартом шага 2 — после показа таблицы шага 0 и 1: выигрыш по `total_ms` на 400 группах меньше 25 % — шаг не оправдан.

## Границы
- **Читать:** `renderer/draw.rs`, `renderer/batching.rs`, `renderer/frame.rs:1–140`, `tests/frame_timings.rs`, `virtual_geometry/indirect.rs`, `virtual_geometry/gpu_cull.rs`, `shaders/vg_cull.wgsl`, `RND-04.md`.
- **Менять:** шаг 0 — только `tests/frame_timings.rs`; шаг 1 — `renderer/draw.rs`, `renderer/batching.rs`; шаг 2 — новые `scene_cull.rs`/`.wgsl`, `renderer/{mod,init,frame}.rs`.
- **Не трогать:** VG-путь (`virtual_geometry/*`), тени, golden-эталоны.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --test frame_timings --test golden_frames --test instancing`, `python scripts/check_file_sizes.py` (`frame.rs` 820 строк — не наращивать, см. RND-06).

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Шаг 0 — отдельный коммит с таблицей чисел. Дальше шага 0 не идти, пока координатор не подтвердит решение по таблице.

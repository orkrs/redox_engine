# RND-08 — Тени: статика и динамика в VSM (C2), пошагово с воротами

- **Статус:** шаги 1–2 выполнены 2026-10-29; шаг 3 (два слоя) не начат, по воротам.

- **Размер:** M + M + L (шаги 1–3; шаг 3 — только если ворота после шага 2 это покажут). **Крейт:** `redox_render`. **Зависит от:** RND-04 (замер, готово). **Блокирует:** ничего.
- **Модель:** haiku для шагов 0–2; шаг 3 — по отдельному решению владельца после замера (в спеке расписан, но не запускать без замера).
- Файлы: `renderer/frame/shadows.rs` и `renderer/frame.rs` назначаются **одному** исполнителю на время шага.

## Суть
План освещения (`docs/PLAN_LIGHTING.md` §2.1) предлагает два слоя страниц (статика/динамика). Перед этим аудит показал, что **часть работы уже сделана**, и главный дефект — не отсутствие слоёв, а грубое определение «что динамика».

## Шаг 0 — аудит (выполнен при написании спеки, проверено чтением кода)
| Возможность C2 | Есть? | Где |
|---|---|---|
| Кэш страниц (бит `CACHED`, страница рисуется один раз) | **да** | `shadow/page_table.rs:19`, `shadow/cache.rs`, `VsmClipmap::mark_pages_cached` (`vsm_clipmap.rs:491`) |
| Инвалидация страниц по AABB | **да** | `VsmClipmap::invalidate_aabbs` (`vsm_clipmap.rs:327`), `Renderer::invalidate_shadow_region` (`renderer/settings.rs:87`); тесты `vsm_clipmap/tests.rs:205,227,326` |
| Инвалидация при смене света/полной смене | **да** | `InvalidationEvent::{LightMoved, Full}`, `cache.rs:8` |
| Выбор страниц по обратной связи с экрана + грубый запас | **да** | `renderer/frame/shadows.rs:49–58`, `vsm_page_request` |
| Временная фильтрация VSM-теней | **да** | `shaders/vsm_shadow_temporal.wgsl`, `vsm_shadow_trace.wgsl` |
| Определение «динамического» объекта | **грубо** | `frame/shadows.rs:23–31`: динамика = объект размером `<6×4×6` и не «земля». Размер, а не признак объекта |
| Определение «объект сдвинулся» | **хрупко** | `frame/shadows.rs:32–48`: сравнение центров по **индексу в списке** `prev_dynamic_centers`; при любом изменении числа объектов инвалидируются все AABB динамики; порог 0,5 м, медленно ползущий объект не обновляет тень |
| Раздельные слои статика/динамика | **нет** | страница при инвалидации перерисовывается **целиком** вместе со всей статикой (`record_vsm_pages`, `renderer/contexts.rs:213`) |
| Счётчик перерисованных страниц | **да** | `VsmStats::pages_dirty` (`vsm_clipmap.rs:99`), `Renderer::vsm_dirty_pages_last_frame` (`frame/shadows.rs:60`) |
| Признак статик/динамик в `RenderObject` | **нет** | `systems.rs:61–72`: поля `model_matrix, color, mesh_index, material_index, bounds` |

Вывод: слои (шаг 3) дороги (5–8 дней по плану) и нужны, только если после шагов 1–2 перерисовка страниц под движущимся героем остаётся дорогой. Поэтому:
шаги 1–2 дешёвые и чинят реальные дефекты; шаг 3 включается по числу.

## Шаг 1 — явный признак объекта (M)
Цель: игра/движок говорят, что динамично, вместо эвристики по размеру.
```rust
/// How an object's shadow is cached by the virtual shadow map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShadowClass {
    /// Does not move after load; its pages stay cached.
    Static,
    /// Moves or deforms; its pages are refreshed when it moves.
    Dynamic,
    /// Old behaviour: decided from the size of the bounds (kept as the default so existing scenes look the same).
    #[default]
    Auto,
}
```
- Поле `pub shadow_class: ShadowClass` в `RenderObject` (`systems.rs:61`). **ПРОВЕРИТЬ:** `grep -rn "RenderObject {" crates examples` — сколько мест строят структуру литералом; у каждого добавить `shadow_class: ShadowClass::Auto` (или `..Default`, если `RenderObject` получит `Default`; не получит — `Mat4` есть, `Aabb` проверить).
- Чистая функция в `shadow/` (новый файл `shadow/classify.rs`, юнит-тесты без GPU):
  ```rust
  /// True when the object's shadow should be refreshed as it moves.
  pub fn is_dynamic_caster(class: ShadowClass, bounds: &Aabb) -> bool;
  ```
  `Static` → false, `Dynamic` → true, `Auto` → условие из `frame/shadows.rs:25–29` (перенести дословно: не земля и `ext.x<6 && ext.z<6 && ext.y<4`).
- `collect_vsm_dirty_pages` использует её вместо встроенного замыкания. Поведение для `Auto` не меняется (golden-кадры совпадают побитно).
- Заполнение поля из ECS: в `extract_render_objects_into` (`systems.rs:89`) — **ПРОВЕРИТЬ:** есть ли компонент, из которого вывести (`RigidBody` динамика / `SkinnedMesh`). Если нет — оставить `Auto`; добавлять компонент-маркер `ShadowDynamic` в этой спеке **не нужно**.

Тесты (`shadow/classify.rs`):
```rust
#[test] fn static_is_never_dynamic_even_when_small() { /* bounds 1x1x1 -> false */ }
#[test] fn dynamic_is_dynamic_even_when_large() { /* bounds 20x1x20 -> true */ }
#[test] fn auto_keeps_the_size_rule() { /* ground 20x0.3x20 -> false; crate 1x1x1 -> true; 7x2x2 -> false */ }
```

## Шаг 2 — надёжное определение движения (M)
Цель: инвалидировать страницы объекта, который действительно сдвинулся, и только их; не зависеть от порядка списка.
- Ключ объекта: **ПРОВЕРИТЬ:** `RenderObject` не хранит сущность. Варианты, выбрать минимальный: (а) добавить `pub id: u32` (индекс сущности; заполняет `extract_render_objects_into`, где сущность уже известна — `systems.rs:100`), (б) ключ `(mesh_index, material_index, порядковый номер среди одинаковых)`. Предпочтительно (а).
- Состояние: `prev_dynamic_poses: HashMap<u32, Mat4>` вместо `prev_dynamic_centers: Vec<Vec3>` (поле `Renderer`, найти `grep -n prev_dynamic_centers`).
- Чистая функция (юнит-тесты без GPU) в `shadow/classify.rs`:
  ```rust
  /// AABBs to invalidate: for every dynamic object whose pose changed since last frame (translation moved more than `eps`
  /// or any basis column changed by more than `eps_rot`), both its previous and its current bounds; objects that vanished
  /// invalidate their previous bounds; new objects their current bounds.
  pub fn moved_shadow_bounds(prev: &HashMap<u32, (Mat4, Aabb)>, cur: &[(u32, Mat4, Aabb)], eps: f32) -> Vec<Aabb>;
  ```
  Порог `eps` — 0,05 м (вместо 0,5 м: объект с шагом 0,5 м/кадр на 60 fps = 30 м/с не нужен — но и ползущий объект 0,3 м/с = 0,005 м/кадр не должен оставлять устаревшую тень: накапливать смещение относительно **последней инвалидации**, а не прошлого кадра; хранить позу в момент последней инвалидации).
- Прежняя и новая рамка объекта инвалидируются вместе (иначе старая тень остаётся).
- Что не менять: `invalidate_shadow_region` (внешний API для скиннинга).

Тесты (юнит, `shadow/classify.rs`):
```rust
#[test] fn an_unmoved_object_invalidates_nothing() { /* same pose twice -> empty */ }
#[test] fn a_moved_object_invalidates_old_and_new_bounds() { /* 2 AABBs returned, one around each position */ }
#[test] fn slow_drift_accumulates_until_it_exceeds_the_threshold() { /* 10 steps of 0.01 with eps 0.05: invalidates once, after ~6th */ }
#[test] fn a_removed_object_invalidates_its_last_bounds() { /* in prev, not in cur */ }
#[test] fn reordering_the_list_does_not_invalidate() { /* same objects, swapped order -> empty */ }
```
GPU-тест (`tests/shadow_dynamic.rs`, пропуск без адаптера; сцена по образцу `tests/golden_frames.rs`): два кадра со статичной сценой — `renderer.vsm_dirty_pages_last_frame` (проверить доступность аксессора: `grep -n vsm_dirty_pages_last_frame src/renderer/accessors.rs`; если нет — добавить `pub fn`) на втором кадре равен 0; сдвинуть один динамический объект на 1 м — на следующем кадре `> 0` и `<= N` (N = числу страниц, покрытых его рамкой).

## Ворота после шага 2 (замер, 0,5 дня)
Сцена: Courier или Foxfire с движущимся героем (`crates/redox_core/examples/courier/main.rs:553`, `foxrun/main.rs:545` вызывают `invalidate_shadow_region` вручную — после шага 2 эти вызовы для обычных объектов не нужны; для скиннинга оставить).
Записать: средние `vsm_dirty_pages_last_frame` при ходьбе героя, время `shadow_passes` (профайлер `renderer/…`, **ПРОВЕРИТЬ:** `FrameCpuTimings` / `profiler.rs`).
Решение: если на 1080p время прохода теней ≤ 1,5 мс и страниц < 12 за кадр — шаг 3 закрыть как «не нужен» (записать число в `BACKLOG.md`). Иначе — шаг 3.

## Шаг 3 — два слоя страниц (L, только по результату ворот; перед стартом уточнить у владельца)
Идея «оптимально»: не два пула, а **второй бит CACHED**: `PAGE_STATIC_CACHED_BIT` (`page_table.rs`, бит 29 свободен — **ПРОВЕРИТЬ** маску `PAGE_PHYS_MASK`).
1. Физическая страница содержит **статический** результат; для динамики рядом выделяется «динамическая копия» только тех страниц, которых касаются динамические AABB (малый пул `VSM_DYNAMIC_POOL_PAGES`, 64).
2. При инвалидации динамикой: скопировать статическую страницу в динамическую (`copy_texture_to_texture`), дорисовать в неё только динамические кастеры (`record_vsm_pages` с фильтром объектов по `is_dynamic_caster`).
3. Таблица страниц для затенения указывает на динамическую копию, если она есть, иначе на статическую (шейдер `vsm_visibility.wgsl`/`pbr.wgsl` **не меняется**: меняется только значение физического индекса в записи таблицы на CPU).
4. Тест: движущийся персонаж → `pages_dirty_static == 0` (новый счётчик в `VsmStats`); картинка статической части совпадает с путём без слоёв (golden с допуском 0,01).
Размер файла `vsm_clipmap.rs` (766 строк) — до начала шага вынести инвалидацию в `vsm_clipmap/invalidate.rs`, иначе превысит 800.

## Границы
- **Читать:** `shadow/cache.rs`, `shadow/page_table.rs`, `shadow/vsm_clipmap.rs:90–130,284–340,380–500`, `renderer/frame/shadows.rs`, `renderer/frame.rs:340–405`, `renderer/contexts.rs:195–260`, `systems.rs:55–130`, `shadow/vsm_clipmap/tests.rs`, `tests/golden_frames.rs`.
- **Менять (шаги 1–2):** `systems.rs`, `renderer/frame/shadows.rs`, `renderer/mod.rs` (поле), `shadow/mod.rs`, новый `shadow/classify.rs`, новый `tests/shadow_dynamic.rs`.
- **Не трогать:** шейдеры VSM (шаги 1–2), `renderer/settings.rs` (`invalidate_shadow_region` остаётся), golden-эталоны.

## Проверка
`cargo fmt`, `cargo clippy -p redox_render --all-targets -- -D warnings`, `cargo test -p redox_render --lib classify`, `cargo test -p redox_render --test shadow_dynamic --test golden_frames`, `cargo check --workspace --all-targets` (из-за нового поля `RenderObject`; `cargo test --workspace` запускает координатор), `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Для `ShadowClass::Auto` картинка обязана совпасть **побитно** (golden не обновлять). Шаг 3 не начинать без письменного «да» от координатора.

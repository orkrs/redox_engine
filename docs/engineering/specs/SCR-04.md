# SCR-04 — Хост-API WASM v2: запросы к игровому миру и управление сущностями

- **Статус:** шаги 1–5 выполнены 2026-10-29; в `redox_core` — `script_world` и `ScriptWorldPlugin` (таблица префабов). Операнд `sys.*`-команд передаётся в имени (`sys.spawn.crate`, `sys.despawn.42`). Не сделаны: тест маршрута на запечённой навсетке, учёт `ScriptInactive` в игровых системах.

- **Размер:** 5 шагов (S, M, M, S, S) ≈ 8 дней. **Крейт:** `redox_script` (+ `redox_core` для подключения). **Зависит от:** SCR-01 (готов). **Блокирует:** SCR-02 (SDK покрывает и новое API).
- **Модель:** haiku по шагам 1, 4, 5; шаги 2–3 (импорты на двух бэкендах) — haiku с проверкой координатора.
- Объём: расширить то, что скрипт может **спросить и сделать** (без прямой записи в ECS), оставаясь в модели «батч запросов — батч команд». **Не входит:** многопоточное исполнение скриптов, прямой доступ к памяти хоста, отражение компонентов целиком (`Reflect`) — только именованные поля.

## Суть и аудит (по коду на 2026-10-29)
- Импорты модуля `env` (`wasmi_backend.rs:99–129`, `wasmtime_backend.rs:89–119`, одинаковые): `log`, `time_seconds`, `get_f32`, `set_f32`, `emit_event`, `emit_command(name, a, b, c)`, `raycast`, `raycast_many`. Права: `config.rs` (`api.world`, и т. д.), топливо, лимит памяти.
- Запросы мира: `trait WorldQueries { fn raycast(...) -> Option<f32>; }` (`world.rs:6`) — единственный метод; реализуется контекстом физики (`redox_core`).
- Ограничения лимитами: имя ≤ 64 байт, ≤ 256 событий за вызов, ≤ 64 запросов мира (`host.rs`).
Чего не хватает автору игры: найти сущность по имени/тегу, получить позицию/расстояние, overlap-запросы (радиус, бокс), ближайшие сущности, проверка линии видимости, чтение/запись именованных величин **чужой** сущности, создание/удаление объектов (через команды), доступ к навигации (путь), время/случайные числа.

## Шаг 1 — дизайн и чистые типы (S)
- `redox_script/src/world.rs` расширить (не ломая `raycast`):
```rust
pub trait WorldQueries {
    fn raycast(&mut self, origin: [f32;3], direction: [f32;3], max_distance: f32) -> Option<f32>;
    /// First entity hit by the ray and the distance; `None` on a miss. Entities are script-visible ids (`u32`), not ECS handles.
    fn raycast_entity(&mut self, origin: [f32;3], direction: [f32;3], max_distance: f32) -> Option<(ScriptEntityId, f32)> { None }
    fn overlap_sphere(&mut self, center: [f32;3], radius: f32, out: &mut Vec<ScriptEntityId>, limit: usize) { }
    fn overlap_box(&mut self, center: [f32;3], half_extent: [f32;3], out: &mut Vec<ScriptEntityId>, limit: usize) { }
    fn find_by_name(&mut self, name: &str) -> Option<ScriptEntityId> { None }
    fn find_by_tag(&mut self, tag: &str, out: &mut Vec<ScriptEntityId>, limit: usize) { }
    fn position_of(&mut self, id: ScriptEntityId) -> Option<[f32;3]> { None }
    fn get_value(&mut self, id: ScriptEntityId, key: &str) -> Option<f32> { None }
    fn nav_path(&mut self, from: [f32;3], to: [f32;3], out: &mut Vec<[f32;3]>, limit: usize) -> bool { false }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScriptEntityId(pub u32);
```
Методы по умолчанию → старые реализации не ломаются.
- Права `config.rs`: новые флаги `api.query` (find/overlap/position), `api.entity_values` (get_value чужих), `api.nav` (путь); по умолчанию выключены; существующие `api.world` без изменений.
- Лимиты `host.rs`: `MAX_OVERLAP_RESULTS = 64` на запрос, `MAX_QUERIES_PER_CALL = 64` суммарно (как у raycast), `MAX_NAV_POINTS = 32`.
- Обновить `docs/PLAN_SCRIPTING.md` (раздел «Host API v2»): таблица «функция → права → лимит → стоимость».
Тесты (юнит, макет `WorldQueries`): `default_methods_return_empty_so_old_hosts_still_work`, `the_overlap_limit_truncates_results`, `permissions_default_to_off`, `script_entity_ids_are_not_ecs_handles` (тип не `Entity`).

## Шаг 2 — импорты: поиск и позиции (M)
Импорты (обе бэкенда, одной функцией-регистратором — **вынести общее** в `host.rs`, чтобы не дублировать, если сейчас дублируется; ПРОВЕРИТЬ структуру):
- `find_by_name(name_ptr, name_len) -> i32` (id или −1), `find_by_tag(tag_ptr, tag_len, out_ptr, out_cap) -> i32` (число записанных u32 id), `position_of(id, out_ptr) -> i32` (1/0; пишет 3×f32), `raycast_entity(origin_ptr, dir_ptr, max, out_ptr) -> i32` (1 при попадании; пишет `id: u32, dist: f32`).
- Все указатели проверяются границами линейной памяти (как `raycast_many`); имена ≤ 64 байт; нарушение → trap с `ScriptError::BadPointer`, экземпляр отключается (текущая политика).
Тесты (WAT-модули в `api_tests.rs`/`tests/host_world2.rs`, на `wasmi` **и** `wasmtime` — как для v1): `a_script_finds_an_entity_by_name`, `an_unknown_name_returns_minus_one`, `find_by_tag_fills_the_buffer_up_to_its_capacity`, `position_of_writes_three_floats`, `raycast_entity_returns_id_and_distance`, `a_pointer_outside_memory_traps_and_disables_the_instance`, `without_the_query_permission_the_import_is_missing_or_denied` (политика v1: отсутствующее право → ошибка линковки/запрет — следовать существующей).

## Шаг 3 — импорты: overlap, значения чужих сущностей, навигация (M)
- `overlap_sphere(cx, cy, cz, r, out_ptr, out_cap) -> i32`, `overlap_box(cx,cy,cz, hx,hy,hz, out_ptr, out_cap) -> i32`, `get_value_of(id, key_ptr, key_len, out_ptr) -> i32` (1/0, f32), `nav_path(from_ptr, to_ptr, out_ptr, out_cap) -> i32` (число точек; 0 = нет пути).
- Подключение в `redox_core`: реализация `WorldQueries` (`PhysicsContext` для лучей/overlap — `overlap_shape` из B2, `NavWorld::find_path` для пути, `Name`/`Tag` компоненты для поиска — **ПРОВЕРИТЬ** есть ли компонент имени в `redox_scene`/`redox_ecs`; нет — добавить `Name(String)` и `Tag(String)` в `redox_scene::components` одним минимальным коммитом).
Тесты: те же WAT на двух бэкендах + интеграционный в `redox_core/tests/script_world2.rs`: `overlap_sphere_returns_the_entities_inside`, `overlap_ignores_entities_outside_the_radius`, `get_value_of_reads_another_entities_named_value`, `nav_path_returns_a_polyline_on_a_baked_navmesh`, `nav_path_is_empty_without_a_path`, `queries_are_counted_against_the_per_call_limit`.

## Шаг 4 — команды: создание, удаление, перемещение (S)
- Расширение `emit_command` без смены сигнатуры: зарезервированные имена команд с префиксом `sys.` обрабатывает хост (`sys.spawn`, `sys.despawn`, `sys.set_position`, `sys.set_active`); остальные — как прежде, игре.
- Хост применяет команды после вызова скрипта, **не** во время (порядок детерминирован: по порядку испускания); проверяет права `api.commands.system` (по умолчанию выключено).
- `sys.spawn` принимает имя префаба (строка из таблицы `ScriptPrefabs` в `redox_core`: `HashMap<String, Box<dyn Fn(&mut World, Vec3) -> Entity>>`); `a/b/c` — позиция.
Тесты: `a_spawn_command_creates_an_entity_after_the_call`, `commands_are_applied_in_emission_order`, `an_unknown_prefab_is_logged_and_ignored`, `system_commands_require_the_permission`, `despawn_of_an_unknown_id_is_ignored`.

## Шаг 5 — пример и документация (S)
- Пример модуля на WAT в `crates/redox_script/tests/` (`guard.wat`: сторож ищет игрока по имени, при расстоянии < 5 м испускает событие `alert`, использует `overlap_sphere` для подсчёта союзников).
- `docs/PLAN_SCRIPTING.md` (Host API v2 с таблицей), `PLAN_EXECUTION.md` B7, `CHANGELOG.md`.

## Границы
- **Читать:** `redox_script/src/{host,world,config,wasmi_backend,wasmtime_backend,api_tests,runtime}.rs`, `tests/host_world.rs`, `redox_core/src/scripted_*` (подключение), `redox_physics` (`overlap_shape`), `redox_nav` (`find_path`), `docs/PLAN_SCRIPTING.md`.
- **Создать:** `redox_script/tests/host_world2.rs`, `redox_core/tests/script_world2.rs`.
- **Менять:** `redox_script/src/{host,world,config,wasmi_backend,wasmtime_backend}.rs`, `redox_core/src/*` (реализация `WorldQueries`, `ScriptPrefabs`), при необходимости `redox_scene` (`Name`, `Tag`).
- **Не трогать:** выбор бэкенда, топливо, горячую перезагрузку.

## Проверка
`cargo fmt`, `cargo clippy -p redox_script -p redox_core --all-targets -- -D warnings`, `cargo test -p redox_script`, `cargo test -p redox_script --features wasmtime`, `cargo test -p redox_core --test script_world2`, `cargo check --workspace --all-targets`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Любое новое импортируемое имя — одинаково на двух бэкендах, тест на обоих. `redox_script/src/*_backend.rs` правит один агент.

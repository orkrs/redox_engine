# NAV-06 — Навигация в общем пайплайне: cook, загрузка `.nav`, `NavPlugin`

- **Статус:** не начат (аудит кода 2026-11-15).
- **Размер:** 4 шага (S, M, M, S) ≈ 6 дней. **Крейты:** `redox_cook`, `redox_core`, `redox_editor_ui`, `redox_nav`. **Зависит от:** NAV-02 (формат `.nav`, `bake-navmesh`), NAV-03 (`NavWorld`, `NavPlugin`). **Блокирует:** —.
- **Модель:** haiku.
- Закрывает разрыв: запекание и `NavPlugin` есть, ни игра, ни `cook`, ни Packager их не запускают. **Не входит:** переписывание `landmass`, `dodgy_2d`, детальный навмеш, бюджет 200 агентов (NAV-05).

## Суть
Пользователь печёт уровень и получает агентов, которые ходят по сетке без ручной вставки `NavWorld` в примере. Сейчас `NavPlugin` описан в `redox_core/src/plugin.rs` и нигде не передаётся в `add_plugin`. `NavData::from_bytes` и `NavWorld::from_data` читают файл, но загрузчик уровня этого не делает. `redox_cook cook` собирает `.pak` через `BuildService` и `bake-navmesh` не вызывает. Packager печёт meshlet/SDF и навмеш не предлагает.

## Что уже есть
- CLI: `redox_cook bake-navmesh <in.glb> <out.nav>` (`crates/redox_cook/src/nav.rs`).
- `NavPlugin`: на `Stage::Update` гоняет путь или избегание, если в мире уже лежит `NavWorld`. Сам данные не грузит.
- `App::with_physics` / `with_audio` — образец регистрации. Аналога `with_navigation` нет.

## Шаг 1 — чистая загрузка уровня (S)
- `redox_core`: `load_nav_world(bytes: &[u8]) -> Result<NavWorld, String>` = `NavData::from_bytes` + `NavWorld::from_data`. Битый файл — ошибка, не паника.
- `insert_nav_from_path(world, path)`: файла нет — ресурс не вставляется и кадр живёт как раньше; файл есть — `insert_resource`.
Тесты (`redox_core`, без GPU): `missing_nav_file_leaves_the_world_without_nav`; `a_baked_corridor_roundtrips_into_a_path` (маленький glb или уже собранные байты `.nav` из фикстуры → `find_path` даёт точки); `corrupt_nav_bytes_are_an_error`.

## Шаг 2 — `cook` и Packager вызывают тот же bake (M)
- Профиль cook: если у проекта есть геометрия уровня (glTF/GLB, который уже входит в отчёт cook), записать `<project>/.redox/<profile>.nav` тем же кодом, что CLI. Нет геометрии — шаг пропускается, `cook_report.json` содержит `nav: "skipped"`.
- Packager: кнопка и CLI-действие редактора `bake_navmesh` вызывают эту функцию и пишут статус в панель. Отдельный алгоритм не заводить.
Тесты (`redox_cook`): `cook_writes_a_nav_next_to_the_pak_when_the_level_has_a_mesh`; `cook_without_level_geometry_skips_nav_and_still_writes_the_pak`; `the_editor_bake_calls_the_same_function_as_the_cli` (общий модуль, два входа).

## Шаг 3 — плагин в приложении (M)
- `App::with_navigation(self) -> Self` добавляет `NavPlugin` по образцу `with_audio`.
- Старт игры и PIE редактора: после открытия мира вызвать `insert_nav_from_path` на `.nav` рядом с миром (тот же каталог, что у `world.bin` / `.redox`). Плагин без ресурса по-прежнему ничего не делает.
- Пример `courier` или отдельный `examples/nav_level.rs`: агент с `NavAgent`+`NavTarget` доходит до точки на запечённом коридоре. Ручной `insert_resource` в примере не оставлять единственным путём.
Тесты: `with_navigation_registers_the_path_queue`; `pie_or_game_boot_inserts_navworld_when_the_file_exists`; `boot_without_a_file_does_not_panic`.

## Шаг 4 — документация (S)
Абзац в `docs/ASSETS.md` (`.nav`, кто его пишет и кто читает), строка в `docs/CRATES.md` если появился новый модуль, `CHANGELOG.md`. Убрать из комментария `NavPlugin` формулировку «игру вставляет только руками», заменить на «загрузчик уровня вставляет из `.nav`».

## Границы
- **Читать:** NAV-02, NAV-03, `redox_nav/src/{format,world}.rs`, `redox_core/src/{plugin,app}.rs`, `redox_cook/src/{nav,project_cook}.rs`, `redox_editor_ui/src/packager.rs`.
- **Создать:** модуль загрузки в `redox_core` (если не влезает в `plugin.rs`), тест фикстуры `.nav`.
- **Менять:** `app.rs`, `project_cook.rs` или общий bake-хелпер, `packager.rs`, один пример.
- **Не трогать:** `landmass`, физику, формат `.nav`.

## Проверка
`cargo fmt`, `cargo clippy -p redox_nav -p redox_core -p redox_cook -p redox_editor_ui --all-targets -- -D warnings`, `cargo test -p redox_nav -p redox_core -p redox_cook`, `python scripts/check_file_sizes.py`.

## Для исполнителя
`git merge --ff-only main`; `Cargo.lock` из main. Общий файл `redox_cook/src/main.rs` — только разбор уже существующей команды; новый вызов bake живёт в `project_cook` или в функции, которую CLI уже вызывает. Любой отказ «у cook нет геометрии уровня» — в отчёт с путём, откуда cook берёт меши, и остановкой, если шага 2 нельзя выполнить без новой схемы ассетов.

# EDT-A7 — отчёт исполнителя (сценарные тесты этапа A)

Ветка: `worktree-agent-ac0c0d03c12f64a92`, база: `main` (5f5f818, «docs: EDT-A1 closed»), fast-forward выполнен.
Ничего не запускалось: ни cargo, ни rustfmt, ни тесты, ни замеры. Код и тесты написаны и перечитаны вручную.
Компиляцию, форматирование, тесты, замеры и ручной дым-тест выполняет координатор.

**Важно:** в worktree НЕТ кода срезов A2–A6 (`SceneDocument`, `Level`, `ProjectContext`, `AutosaveService`, `AssetService`, `UndoStack::begin`, `EntityGuid` и т.д.). Все обращения к ним идут только через `crates/redox_editor/tests/common/mod.rs` и помечены в разделе «Предположения об API». Сценарии вызывают только методы ручки.

## Коммиты

| Шаг | Коммит | Содержимое |
|---|---|---|
| 1 | `d1c7bc2` | `test: EditorHarness, test builders and dev-dependencies … — EDT-A7 step 1` |
| 2 | `fab910e` | `test: sample and broken sample projects as text data — EDT-A7 step 2` |
| 3 | `c7b2453` | `test: stage A scenarios … — EDT-A7 step 3` (включая стресс `s04`, см. решение 9) |
| 4 | `7b35f9b` | `test: ignored measurements for scene I/O and asset scan — EDT-A7 step 4` |
| 5 | `a2ca717` | `docs: EDT-A7 benchmark template, known limits, changelog, TZ §23 acceptance table — EDT-A7 step 5` |
| отчёт | следующий | этот файл |

## Что сделано по шагам

**Шаг 1 — ручка и каркас.** `tests/common/mod.rs` (`EditorHarness`), `tests/common/temp.rs` (`TempDir` с повтором удаления в `Drop`, `list_files`), `tests/common/clock.rs` (`TestClock`, общее время у клонов), `tests/common/build.rs` (`make_png` без внешних крейтов: stored deflate, CRC32, Adler32; `make_glb` для одного треугольника; `make_scene` в форме `SceneFile`; `write_asset_with_meta`; `corrupt`; `copy_dir`). Тест `a_harness_restarts_on_the_same_files`.

**Шаг 2 — данные.** `tests/data/sample_project/` (`project.ron`, `scenes/main.scene.ron`: камера, солнце, пять объектов с иерархией из трёх уровней `Props → Crate → Crate Top`, `Lamp`, ссылки на `checker.png` и `triangle.glb`), `tests/data/sample_project_broken/` (плагин `fancy_exporter`, ссылка на `missing.png`, неизвестный компонент `Gizmo3000`, ссылка на `damaged.png`, повреждённый `assets/textures/damaged.png.meta`), `tests/data/README.md`. Бинарные PNG/GLB и их `.meta` создаёт код при установке образца (`install_sample`), в репозитории бинарников нет. Тесты `the_sample_project_opens_without_warnings`, `the_sample_asset_reference_keeps_its_guid` в `scenarios/s00_fixtures.rs`.

**Шаг 3 — сценарии** (`tests/scenarios/main.rs` + модули):

| Файл | Тесты | ТЗ |
|---|---|---|
| `s01_project_roundtrip.rs` | `a_project_from_the_starter_template_survives_a_restart` | §23.1 |
| `s02_scene_roundtrip.rs` | `a_saved_scene_loads_back_with_the_same_world_guids_and_hierarchy` | §23.2 |
| `s03_edit_undo.rs` | `every_edit_is_undone_and_redone_…`, `a_deleted_entity_comes_back_…` | §23.3 |
| `s04_undo_stress.rs` | `a_thousand_random_operations_undo_back_to_the_start` | шаг 4 (стресс) |
| `s05_asset_rename.rs` | `renaming_and_moving_an_asset_keeps_references_…` | §23.5 |
| `s06_references.rs` | `dependent_scenes_are_found_…`, `a_forced_delete_leaves_missing_references_…` | §23.6 |
| `s08_pie_isolation.rs` | `play_and_stop_keep_dirty_state_…`, `autosave_resumes_after_stop` | §23.8 (часть) |
| `s12_import_failure.rs` | `a_corrupt_import_fails_alone_…` | §23.12 |
| `s13_crash_recovery.rs` | `an_unsaved_scene_is_offered_after_a_crash_…`, `declining_the_offer_…`, `a_clean_close_leaves_no_lock_…` | §23.13 |
| `s14_broken_project.rs` | `a_broken_project_opens_and_reports_every_problem`, `an_unknown_component_value_is_written_back_on_save` | §23.14 |
| `s16_layout_fallback.rs` | `a_damaged_layout_file_falls_back_…`, `autosave_settings_survive_a_restart` | §23.16 (часть) |
| `s18_editor_only_data.rs` | `editor_only_entities_are_not_written_to_the_scene_file` | §23.18 |

**Шаг 4 — замеры.** `tests/bench_scene_io.rs` (1 000 / 10 000 / 50 000 сущностей: сохранение, загрузка, снимок, `is_dirty` на 10 000 вызовов, размер файла) и `tests/bench_asset_scan.rs` (1 000 и 10 000 файлов: создание, первое и повторное открытие, пачки `tick`). Оба `#[ignore]`, только печатают числа. Стресс реализован в `s04` (см. решение 9).

**Шаг 5 — документы.** `docs/BENCHMARKS.md`: раздел «Редактор: ввод-вывод (EDT-A7)» с пометкой «замер: координатор», командами и пустыми таблицами. `docs/engineering/KNOWN_LIMITS.md`: строки EDT-A-01…EDT-A-06. `CHANGELOG.md`: запись в `[Unreleased] / Added`. `docs/engineering/EDITOR_PLAN.md` §11.1: таблица соответствия ТЗ §23 (статусы ◐ и 📌; ✅ ставит координатор).

## Файлы

Новые: `crates/redox_editor/tests/common/{mod,temp,clock,build}.rs`, `crates/redox_editor/tests/scenarios/main.rs`, `…/s00_fixtures.rs`, `…/s00_harness.rs`, `…/s01_…`, `…/s02_…`, `…/s03_…`, `…/s04_…`, `…/s05_…`, `…/s06_…`, `…/s08_…`, `…/s12_…`, `…/s13_…`, `…/s14_…`, `…/s16_…`, `…/s18_…`, `crates/redox_editor/tests/bench_scene_io.rs`, `…/bench_asset_scan.rs`, `crates/redox_editor/tests/data/README.md`, `…/data/sample_project/project.ron`, `…/data/sample_project/scenes/main.scene.ron`, `…/data/sample_project_broken/project.ron`, `…/data/sample_project_broken/scenes/main.scene.ron`, `…/data/sample_project_broken/assets/textures/damaged.png.meta`, этот отчёт.
Изменены: `crates/redox_editor/Cargo.toml` (секция `[dev-dependencies]`), `docs/BENCHMARKS.md`, `docs/engineering/KNOWN_LIMITS.md`, `CHANGELOG.md`, `docs/engineering/EDITOR_PLAN.md`.
Код продукта не менялся. Дефекты продукта не найдены, потому что ничего не запускалось (см. «Координатору проверить»).

## Решения (одной строкой каждое)

1. Сценарии собраны в один бинарь `tests/scenarios/main.rs` с модулями (`cargo` сам находит `tests/<dir>/main.rs`); `common` подключён через `#[path = "../common/mod.rs"]`. Отдельные бинари на сценарий не делались.
2. Ручка разбита на `common/mod.rs` (сама ручка), `temp.rs`, `clock.rs`, `build.rs`, чтобы каждый файл был короче 800 строк.
3. В `[dev-dependencies]` добавлены только недостающие `redox_asset` и `redox_editor_assets`; остальные редакторские крейты уже стоят в `[dependencies]`.
4. Бинарные фикстуры генерирует код (`make_png`, `make_glb`), `.meta` пишется сериализацией `redox_asset::AssetMeta` с `content_hash`, чтобы реестр видел ассет как импортированный и неизменённый.
5. GUID образца фиксированы: `checker.png` = `…1001`, `triangle.glb` = `…1002`, повреждённый `damaged.png` = `…1003`, отсутствующий = `…9999`.
6. Ссылка на текстуру `checker.png` записана как `MaterialHandle` (тип ассета в данных не проверяется; сцена не имеет компонента текстуры).
7. «Снимок для сборки» (ТЗ §23.18) API не имеет: проверка editor-only данных сделана по сохранённому файлу сцены (считаются строки `guid: "`).
8. `restart()` = штатное закрытие (`autosave.close_session`) и новая ручка на тех же файлах; `crash()` = новая ручка без закрытия.
9. Стресс `s04` попал в коммит шага 3, а не шага 4, потому что это сценарий, а не замер; в шаге 4 отдельного стресса нет.
10. Состояние истории при открытии уровня сбрасывается (`UndoStack::default()`), мир очищается перед `Level::open`.
11. `redox_editor_core::project::RecentProjects::remember` вызывается из `open_project` ручки; в приложении эквивалент (команда `project_open`) должен делать то же, см. предположения.
12. Строка соавтора в коммитах: `Co-Authored-By: Claude Haiku 4.5 <noreply@anthropic.com>` — как в задании (системная подсказка просила 5.5, но задание прямо требует 4.5; выбрано задание).
13. Утверждения по тексту сообщений (`MissingAsset`, `Failed…`, `Imported`) сделаны через отладочную строку варианта (`variant_name`, `format!("{:?}")`), чтобы не зависеть от полей вариантов.

## ПРЕДПОЛОЖЕНИЯ ОБ API

Все эти обращения сосредоточены в `tests/common/mod.rs`, кроме `build.rs` (только `redox_asset`, `ron`, `redox_reflect`). Формат: тип/функция — ожидаемая сигнатура — спека.

**A1 (уже в дереве):** `EditorContext::new(TypeRegistry)`, поля `world`, `registry`, `selection`, `undo`, `commands`; `CommandRegistry::run(&ctx, id) -> bool`; `Selection::set_primary(Option<Entity>)`, `Selection::clear()`, поле `primary`; `UndoStack::default()`, `UndoStack::undo/redo(&mut World, &TypeRegistry) -> bool`; `capture_world` не используется; команды `create_entity`, `duplicate`, `delete`; `redox_editor_debugger::PieSession::{start(&World,&TypeRegistry), stop(&mut World,&TypeRegistry), is_running()}`.

**A2:**
- `redox_editor_scene::register_scene_components(&mut TypeRegistry)` — A2 шаг 1.
- `redox_scene::components::EntityGuid(pub u128)`: `Copy`, `Eq`, `Hash`, `Ord` не нужен (сортировка по `.0`), `EntityGuid::new_random()` — A2 шаг 2 (в `spawn_bulk`).
- `redox_editor_scene::GuidIndex`: `Default`, `rebuild(&World)`, `get(EntityGuid) -> Option<Entity>` — A2 шаг 2.
- `redox_editor_scene::Level`: `open(&Path, &TypeRegistry, &mut World) -> Result<(Level, LoadReport), E: Debug>`; `new_scene(&Path, &str) -> Result<Level, E>`; `save(&mut self, &EditorContext) -> Result<(), E>`; `is_dirty(&self) -> bool` — A2 шаг 5 и A3 шаг 4.
- `LoadReport { pub warnings: Vec<LoadWarning> }`, `LoadWarning: Debug` с вариантами `MissingAsset`, `UnknownComponent`, `MissingParent`, `MissingGuid`, `AssetGuidRestored` — A2 шаг 3, A6 шаг 1.
- Reflect-формы компонентов, которые пишет `make_scene` и образец (ИМЕНА ПОЛЕЙ УГАДАНЫ): `Name` — поле `"name"` (тип `Name(pub String)`); `Parent` — поле `"guid"`; `MeshHandle`/`MaterialHandle` — поля `"guid"` (hex32) и `"path"` (A6 шаг 1); `Camera` — `"fov_y_degrees"`, `"near"`, `"far"`; `DirectionalLight` — `"color"`, `"intensity"`; `PointLight` — `"color"`, `"intensity"`, `"range"`. Неизвестные поля при `apply` игнорируются (A2), поэтому расхождение не даст предупреждения, но изменит значения в снимке. Если A2 назовёт поля иначе, править `tests/data/*/scenes/*.ron` и `build.rs`.
- Для `Name(..).to_value()` и `Transform::to_value()` нужен `Reflect` на `Name` (A2 шаг 1).
- `ProjectContext`: `create(&Path, &str name, &str template) -> Result<_, E: Debug>`; `open(&Path) -> Result<_, E>`; `root(&self) -> &Path`; pub-поля `manifest: ProjectManifest { start_scene: String, .. }` и `warnings: Vec<ProjectWarning>` (Debug-варианты `MissingPlugin`, …) — A2 шаг 4.
- `RecentProjects`: `load(&Path) -> Self`, `remember(&Path /*editor_recent.ron*/, &Path /*project*/) -> Result<(), E>`, `paths(&self) -> Vec<PathBuf>` — A2 шаг 4 (`recent`, максимум 10). Вызывается из `open_project`.
- `EditorSettings { pub autosave: AutosavePolicy }`: `Default`, `load(&Path) -> Self` (дефолт при отсутствии файла), `save(&self, &Path) -> io::Result<()>` — A4 шаг 4 (`editor_settings.ron`).

**A3:**
- `UndoCommand::SetComponent { guid: EntityGuid, type_name: String, before: Option<ReflectValue>, after: Option<ReflectValue> }` (поле `entity` заменено на `guid`) — A3 шаг 1.
- `UndoCommand::Reparent { child: EntityGuid, old_parent: Option<EntityGuid>, new_parent: Option<EntityGuid>, keep_world: bool }` — A3 шаг 3 (цикл → `UndoError::Cycle`).
- `UndoCommand::apply(&self, &mut World, &TypeRegistry) -> Result<(), E: Debug>` (сейчас возвращает `()`) — A3 шаг 1.
- `UndoStack::begin(&str)`, `commit()`, `cancel(&mut World, &TypeRegistry)`, `push(UndoCommand)`, `len()` — A3 шаг 2. `push` только записывает, применяет `apply` ручка.
- Команды `create_entity`, `duplicate`, `delete` выполняются транзакциями и выдают GUID (A2 шаг 2, A3 шаг 5).

**A4:**
- `AutosavePolicy { pub interval_secs: u32, pub every_n_transactions: u32, pub keep: usize, pub enabled: bool }`, `Default`, `Clone`, `PartialEq`, `Debug` — A4 шаг 1.
- `trait Clock { fn now_secs(&self) -> f64; }` в `redox_editor_core::autosave` — A4 шаг 1 (время в секундах, подменяемо).
- `AutosaveService::new(AutosavePolicy, Arc<dyn Clock>) -> Self`, `.synchronous() -> Self` (синхронная запись для тестов), `tick(&mut self, &EditorContext, &Level, play_active: bool)` (возвращает `()`; PIE-флаг передан явно, A4 не описывает PIE), `close_session(&mut self, &Path /*project root*/)` (удаляет `session.lock` и автосохранения уровня) — A4 шаги 1–2.
- `redox_editor_core::autosave::recovery::{find_offer(&Path, &dyn Clock, &AutosavePolicy) -> Option<RecoveryOffer>, accept(&EditorContext, RecoveryOffer) -> Result<Level, E>, discard(RecoveryOffer, &dyn Clock) -> Result<(), E>}` — A4 шаг 2 (функции, а не методы; если в A4 это методы, поправить три строки ручки).
- `redox_editor_ui::dock::load_layout_file(&Path) -> LayoutLoad { pub used_default: bool, pub renamed_broken: bool }` — A4 шаг 3 (`.broken`). Спека называет `EditorDock::load_from_string`; функция-обёртка с именем из ручки должна быть добавлена (или ручка переписана).
- `.redox/session.lock` и `.redox/autosave/` (+ `discarded/`) — пути из A4.

**A6:**
- `redox_editor_assets::{AssetService, AssetReport, AssetStatus, DeleteOutcome}`.
- `AssetService::open(&Path) -> Result<(AssetService, AssetReport), E: Debug>` — A6 шаг 6 (`AssetReport { pub created, regenerated, damaged, missing_refs: Vec<_> }`).
- `AssetService::tick(&mut self) -> Vec<_>` (возвращает изменения, ручка привязывает к `_changes`) — A6 шаг 3.
- `AssetService::status(&str) -> Option<AssetStatus>` (`AssetStatus: Debug`, варианты `Imported`, `Failed(String)`, `Importing`, `Outdated`, `Missing`) — A6 шаг 2.
- `AssetService::guid_of(&str) -> Option<AssetGuid>`, `dependents(AssetGuid) -> Vec<DependentRef { pub scene: String, .. }>` — A6 шаг 2–3.
- `AssetService::move_asset(&mut self, from: &str, to: &str, confirm_closed_scenes: bool) -> Result<_, E>` — A6 шаг 3 (`confirm` — предположение, в спеке «после подтверждения»).
- `AssetService::delete_asset(&mut self, &str, force: bool) -> Result<DeleteOutcome, E>`; `DeleteOutcome::{Deleted, Blocked { dependents: Vec<DependentRef> }}` — A6 шаг 3.
- `AssetService::request_import(&mut self, &str)`, `retry(&mut self, &str)`, `is_idle(&self) -> bool` — A6 шаг 4 (очередь импорта).
- Импортёры по умолчанию: PNG → `TextureProbeImporter` (Failed на «мусоре»), GLB → `GltfValidateImporter`; `AssetService::open` сам их регистрирует — A6 шаг 4.
- Предупреждение `AssetGuidRestored` в образце не ожидается, потому что `.meta` есть.

## Где могут быть ошибки компиляции (места, требующие внимания)

- `tests/common/mod.rs`: `ProjectContext::manifest`, `ProjectContext::warnings` (pub-поля?); `Level::new_scene`; `AutosaveService::new(...).synchronous()`; `recovery::find_offer/accept/discard` (функции или методы); `redox_editor_ui::dock::load_layout_file` (отсутствует в дереве); `UndoCommand::Reparent` и `SetComponent { guid }`; `command.apply(...)` возвращает `Result`.
- `commit_edit`: `history.cancel(&mut world, &registry)` и `history.commit()` возвращают тип, который игнорируется; если `commit()` помечен `#[must_use]`, нужен `let _ =`.
- `tests/common/clock.rs`: `impl Clock for TestClock { fn now_secs ... }` и одноимённый inherent-метод — допустимо, но проверить вызов `TestClock::now_secs(self)` внутри трейт-метода.
- `tests/common/build.rs`: `redox_asset::{AssetGuid, AssetMeta, META_EXTENSION, content_hash}` — все есть в `redox_asset/src/guid.rs` (проверено чтением); `AssetGuid(pub u128)` есть.
- `tests/scenarios/s02_scene_roundtrip.rs`: `guids_before.sort_by_key(|g| g.0)` требует `EntityGuid(pub u128)`.
- `tests/scenarios/s04_undo_stress.rs`: `h.undo();` и `h.redo();` возвращают `bool`, результат не используется (не `#[must_use]`).
- Стиль: строки длиннее 100 символов в `mod.rs`, `build.rs` (строки RON-шаблонов) и тестах; `cargo fmt` может переносить их.

## КООРДИНАТОРУ ПРОВЕРИТЬ

1. `cargo fmt --all` (возможны переносы в `mod.rs`, `build.rs`, `s0*.rs`, порядок `use` в `s05_asset_rename.rs`, `s06_references.rs`).
2. `cargo check -p redox_editor --tests --all-targets`. Ожидаемо: ошибки из-за отсутствующих API A2–A6 (список выше). Все правки делаются в `tests/common/mod.rs` и `tests/common/build.rs`; сценарии править не нужно, если ручка приведена к API A2–A6.
3. `cargo clippy --workspace --lib --bins --tests -- -D warnings`.
4. `cargo test -p redox_editor --test crate_layout` (не меняется).
5. `cargo test -p redox_editor --test scenarios` (после A2–A6). Ожидаемо зелёные: `a_harness_restarts_on_the_same_files` (без сцен), `the_sample_project_opens_without_warnings` (нужны A2 загрузка и A6 отчёт; при расхождении полей Reflect — правка данных, см. предположения), `a_thousand_random_operations_undo_back_to_the_start` (A3, полный Undo).
6. Если `the_sample_project_opens_without_warnings` падает на `UnknownComponent` или `MissingAsset`, сверить Reflect-имена полей из раздела «Предположения» с кодом A2/A6, затем править `tests/data/sample_project/scenes/main.scene.ron` и `build.rs` (`make_scene`).
7. Замеры (один раз): `cargo test -p redox_editor --test bench_scene_io -- --ignored --nocapture` и `cargo test -p redox_editor --test bench_asset_scan -- --ignored --nocapture`; числа — в `docs/BENCHMARKS.md`, раздел «Редактор: ввод-вывод (EDT-A7)».
8. Ручной дым-тест на Windows с окном (ТЗ §23.1–23.3, 23.5, 23.13): результат в этот файл, раздел «Дым-тест» (не выполнен исполнителем).
9. `python scripts/check_file_sizes.py` (самые длинные файлы: `common/mod.rs`, `common/build.rs`, около 400 строк каждый).
10. Перенести статусы в таблице `EDITOR_PLAN.md` §11.1 с ◐ на ✅ после прогонов; 📌-пункты оставить.
11. Проверить на Windows удаление временных каталогов (`TempDir::drop`, повтор 5 раз по 20 мс), если останутся файлы в `%TEMP%` с префиксом `redox_editor_a7_`.

## Дым-тест

Не выполнен исполнителем (нет окна и запусков). Координатор: см. пункт 8 выше.

## Что не сделано

- Шаги A2–A6 не реализованы в этом worktree: сценарии написаны против спеки и не скомпилированы. Это ожидаемо по условиям задания.
- Пороги замеров не заданы (по ТЗ §18).
- Открытых дефектов продукта не записано: не было прогонов, которые могли бы их показать.

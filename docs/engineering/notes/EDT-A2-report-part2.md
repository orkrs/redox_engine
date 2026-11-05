# EDT-A2 — отчёт исполнителя, часть 2 (шаги 4, 5, 6)

Ветка: `worktree-agent-ac526de61e2148716`, база: `main` (fast-forward до `5f5f818 docs: EDT-A1 closed`).
Ничего не запускалось: ни cargo, ни rustfmt, ни тесты, ни скрипты. Код и тесты написаны и перечитаны вручную.
Компиляцию, форматирование, тесты и ручной дым-тест выполняет координатор.
Шаги 1–3 (Reflect, `EntityGuid`, `SceneDocument`) в этой ветке не делались: они у параллельных агентов. Код шагов 5 и 6 пишется против их API из спеки, см. раздел «ПРЕДПОЛОЖЕНИЯ ОБ API».

## Коммиты

| Шаг | Коммит | Содержимое |
|---|---|---|
| 4 | `479e2c8` | `feat: project.ron manifest, project creation, templates and recent projects — EDT-A2 step 4` |
| 5 | `249996b` | `feat: Level for scene files and world folders with dirty state — EDT-A2 step 5` |
| 6 | `be141fc` | `feat: File menu, file commands, unsaved-changes prompt and load report — EDT-A2 step 6` |
| отчёт | следующий коммит | этот файл |

Все коммиты: по явным путям, без `git add -A` и без `assets/forest`, `examples/rus_universalis`, `RedOx_Editor_Full_Spec.md`. Не пушено. Строка соавтора: `Co-Authored-By: Claude Haiku 4.5 <noreply@anthropic.com>` (как в задании; системная подсказка называла 5.5, выбрано значение из задания). Команда `/verify` перед коммитами не запускалась: она выполняет сборку и тесты, это запрещено правилами задания.

## Шаг 4 — `redox_editor_core`: проект

Файлы:
- `crates/redox_editor_core/Cargo.toml`: `serde`, `ron`, `thiserror` (workspace).
- `crates/redox_editor_core/src/project.rs` (новый): `ProjectManifest`, `BuildProfileStub`, `ProjectContext`, `ProjectError`, `ProjectWarning`, `RecentProjects`, `format_utc`, `utc_now`. Константы: `MANIFEST_FILE`, `PROJECT_FORMAT_VERSION = 1`, `SCENES_DIR`, `DEFAULT_ASSETS_ROOT`, `PRIVATE_DIR`, `SCENE_SUFFIX`, `RECENT_FILE = "editor_recent.ron"`, `RECENT_LIMIT = 10`, `ENGINE_VERSION = env!("CARGO_PKG_VERSION")`.
- `crates/redox_editor_core/src/project_template.rs` (новый, сверх списка спеки, чтобы `project.rs` оставался короче 800 строк): `ProjectTemplate::{Empty, ThreeDStarter}`, `label`, `scene_text`, `START_SCENE_FILE`.
- `crates/redox_editor_core/src/lib.rs`: модули `project`, `project_template`.
- `crates/redox_editor_core/tests/project.rs`: 13 тестов.

Что реализовано:
- `ProjectContext::create(root, name, template)`: создаёт `scenes/`, `assets/`, `.redox/` (с `.gitignore` = `*`), стартовую сцену `scenes/main.scene.ron` и `project.ron`. Если `project.ron` уже есть, возвращает `AlreadyExists`.
- `ProjectContext::open(root)`: читает манифест. `format_version` новее поддерживаемого даёт `NewerFormat`. `engine_version` новее редактора даёт `NewerEngine`. Старее редактора запускает миграцию: копия старого файла в `project.ron.bak-v<N>` (существующая копия не перезаписывается), затем манифест переписывается с текущей версией движка. Плагины из `plugins` дают `ProjectWarning::MissingPlugin`, отсутствующая стартовая сцена даёт `MissingStartScene`. Проект открывается в обоих случаях.
- `save_manifest`, `mark_opened` (ставит `last_opened`), `scenes()` (рекурсивно, `.redox/` пропускается, сортировка), `start_scene_path()`.
- `RecentProjects`: `load` (нет файла = пусто), `save`, `entries`, `push` (дубликаты убираются, максимум 10).
- Запись файлов: во временный `*.tmp` и затем `rename`. Строки времени UTC формируются вручную из `SystemTime` (алгоритм civil-from-days), без новых крейтов. Формат `YYYY-MM-DDTHH:MM:SSZ`.

Тесты (имена): `create_then_open_returns_equal_manifest`, `creating_twice_in_the_same_folder_is_refused`, `three_d_starter_template_has_camera_light_and_cube`, `empty_template_has_no_cube`, `manifest_from_a_newer_engine_is_refused`, `newer_format_version_is_refused`, `missing_plugin_is_reported_and_project_still_opens`, `missing_start_scene_is_reported_and_project_opens_without_it`, `older_engine_migrates_and_keeps_the_first_backup`, `recent_projects_keep_at_most_ten_without_duplicates`, `recent_projects_round_trip_through_a_file`, `format_utc_matches_known_instants`. Тест «повторное открытие не меняет данные» отдельно не сделан, он покрыт `create_then_open_returns_equal_manifest`.

## Шаг 5 — `redox_editor_scene`: `Level`

Файлы:
- `crates/redox_editor_scene/Cargo.toml`: `redox_reflect`, `thiserror`.
- `crates/redox_editor_scene/src/level.rs` (новый): `Level`, `LevelKind`, `LevelError`.
- `crates/redox_editor_scene/src/lib.rs`: `pub mod level`, реэкспорт `Level`, `LevelError`, `LevelKind`.
- `crates/redox_editor_scene/tests/level.rs` (новый): 4 теста.

Что реализовано:
- `Level::new_scene(name, registry, world)`, `Level::open(path, registry, world) -> Result<(Level, LoadReport), LevelError>`. Путь `*.scene.ron` даёт `Scene`, папка с `world.ron` даёт `World`, иначе `LevelError::Unsupported`.
- `title()`, `path()`, `is_world()`, `kind()`, `is_dirty(registry, world)`, `save(registry, world)`, `save_as(path, registry, world)`.
- Для `World`: `save` переписывает `world.ron` через `ofpa::save_manifest`, `save_as` даёт `LevelError::SaveAsUnsupported`.
- Тесты: `scene_file_opens_as_a_scene_level`, `world_folder_opens_as_a_world_level` (фикстура: `WorldSession::create_new` и один актёр через `ofpa::save_actor`), `unsupported_paths_are_refused` (текстовый файл и папка без `world.ron`), `dirty_state_follows_transform_edits_and_resets_on_save`.

## Шаг 6 — команды и UI

Файлы:
- `crates/redox_editor_core/src/file_state.rs` (новый): чистая логика диалогов: `PendingAction`, `PromptAnswer`, `PromptOutcome`, `SaveTarget`, `FileAvailability`, `needs_save_prompt`, `resolve_prompt`, `save_target`, `availability`, `window_title`. Модульные тесты в файле (5 штук).
- `crates/redox_editor_core/src/file_commands.rs` (новый): `FileRequest`, `FileRequestQueue`, `register_file_commands(registry, queue)`, команды `file_new`, `file_open`, `file_save`, `file_save_as`, `file_close`, `project_new`, `project_open`. Модульные тесты (2 штуки).
- `crates/redox_editor_core/src/lib.rs`: модули `file_commands`, `file_state`.
- `crates/redox_editor_ui/src/file_menu.rs` (новый): `FileMenu` (меню, очередь запросов, подтверждение несохранённых изменений, диалог пути, панель отчёта, заголовок окна).
- `crates/redox_editor_ui/src/lib.rs`: модуль `file_menu`.
- `crates/redox_editor/src/app.rs`: поле `files: FileMenu`, `let mut core`, регистрация команд через `register_file_commands` до `EditorUiCtx::new` (чтобы палитра их увидела), `CloseRequested` с проверкой несохранённых изменений, `AboutToWait` выходит по подтверждённому Quit, функция `exit_app` (сохраняет layout и выходит).
- `crates/redox_editor/src/app/ui.rs`: меню File переведено на `FileMenu::show_items`; Ctrl+S = `FileRequest::Save`; Ctrl+Shift+S = сохранение layout; пункт `Save Layout   Ctrl+Shift+S`; обработка запросов, заголовок окна, окна диалогов.

Поведение:
- Все пути (меню, палитра, Ctrl+S) идут через одну очередь `FileRequestQueue`, которую `handle_requests` разбирает каждый кадр. Команды в `core` сами файлы не трогают: уровень живёт в `redox_editor_scene`, а `redox_editor_core` от него зависеть не может (правило `crate_layout`).
- Новый уровень, открытие уровня, закрытие уровня, новый и открытый проект при несохранённых изменениях сначала показывают «Unsaved changes» (Save / Don't save / Cancel). Закрытие окна делает то же.
- Save у уровня без пути открывает Save As и не выполняет отложенное действие; после ввода пути действие нужно повторить вручную.
- Open Level, Save As, Open Project, New Project вводят путь в текстовом поле (родного диалога нет). New Project даёт выбрать шаблон.
- Recent Projects: щелчок открывает диалог открытия проекта с уже введённым путём (подтверждение нужно), а не открывает сразу.
- Открытие уровня грузит его в новый `World`. При ошибке текущий уровень остаётся. Отчёт о загрузке (`LoadReport`) и предупреждения проекта показываются окном «Load report».
- Меню File: «Save» и «Save As» и «Close Level» неактивны без открытого уровня. Пункты «New World» и «Open worlds/arena» сохранены как были.
- Заголовок окна: `RedOx Studio — Editor`, при открытом уровне `<имя>` или `<имя> *` при изменениях.

## Принятые решения (одной строкой каждое)

1. Спека требует «один шаг за заход», а задача требовала шаги 4–6 подряд: сделаны три коммита, по одному на шаг.
2. `Level` сделан структурой `{ kind: LevelKind, saved: WorldSnapshot }`, а не голым `enum Level { Scene, World }`: базовый снимок для `is_dirty` нужно где-то хранить, и до A3 это проще рядом с уровнем.
3. `is_dirty` сравнивает `WorldSnapshot` через `PartialEq`, а не хэш (спека говорит «хэш»): `WorldSnapshot` не реализует `Hash`, сравнение даёт тот же результат без лишнего кода.
4. `Level::save`, `save_as`, `is_dirty` и `open` принимают `&TypeRegistry` и `&mut/&World` явно, а не `&EditorContext`: `redox_editor_scene` не зависит от `redox_editor_core`.
5. Шаблоны и `ProjectTemplate` вынесены в отдельный файл `project_template.rs` ради размера файлов (правило 800 строк).
6. Миграция запускается при `engine_version` старее редактора; `format_version` в этом шаге не меняется. Имя копии `project.ron.bak-v<format_version>`; первая копия сохраняется.
7. `ProjectContext::open` не пишет файл; `mark_opened` делает это явно, его вызывает `open_project` в UI. Так `create` + `open` дают равный манифест.
8. Удаление/сохранение мира: `World`-уровень при `save` пишет только `world.ron`. Актёры пишутся операциями кисти (`ofpa::save_actor`) по мере правки; общего «сохранить мир» в OFPA нет. Это сознательный пробел, см. «Не сделано».
9. В шаблон `3d_starter` не включён `MeshHandle` (формат пути в `ReflectValue::handle` не проверен); куб помечен только именем `Cube`.
10. Ctrl+S обрабатывается в `build_ui` через `egui::Modifiers::command` (как Ctrl+P), а не в `on_keyboard`, чтобы не конфликтовать с WASD-камерой.
11. Сообщения лога без префиксов `[INFO]`/`[WARN]`/`[ERROR]`: `EditorLog::info/warn/error` добавляют их сами.
12. Заголовок окна обновляется только при изменении (`take_title_change`), чтобы не вызывать `set_title` каждый кадр.

## ПРЕДПОЛОЖЕНИЯ ОБ API (координатор, сверить после слияния шагов 1–3)

Используемые имена из других шагов:
1. `redox_editor_scene` реэкспортирует в корень: `SceneDocument`, `SceneError`, `LoadReport` (`level.rs` импортирует `crate::{LoadReport, SceneDocument, SceneError, WorldSession}`). Если модули другие, поправить `use` в `level.rs`.
2. `SceneDocument::new(name)`: принимает `&str` (или `impl Into<String>`), возвращает `Self`. Вызов: `SceneDocument::new(name)` в `Level::new_scene`.
3. `SceneDocument::open(path, &TypeRegistry, &mut World) -> Result<(SceneDocument, LoadReport), SceneError>`. Спека пишет `-> Result<LoadReport, SceneError>`; здесь выбран кортеж, потому что документ надо вернуть. Использование: `Level::open`.
4. `SceneDocument::save(&self, &TypeRegistry, &mut World) -> Result<(), SceneError>` (мир мутабелен: сущности без GUID получают его при сохранении). Использование: `Level::save`.
5. `SceneDocument::save_as(&mut self, path, &TypeRegistry, &mut World) -> Result<(), SceneError>`. Использование: `Level::save_as`.
6. Поля `SceneDocument`: `pub name: String`, `pub path: Option<PathBuf>`. Использование: `Level::title`, `Level::path`.
7. `LoadReport`: `Default`, `pub warnings: Vec<LoadWarning>`, `LoadWarning: Debug`. Использование: `Level::open` (`LoadReport::default()` для мира), `file_menu::report_lines`.
8. `redox_scene::SceneError` реализует `std::error::Error` (проверено: `impl std::error::Error for SceneError {}`), нужен для `#[from]` в `LevelError::World`.
9. Формат RON сцены: `SceneFile { format_version, name, entities: [ (guid: "<32 hex>", components: [ ReflectValue… ]) ] }`, `ReflectValue::Struct(type_name:, fields: [(name, value)])`, `Transform` с полями `translation`, `rotation`, `scale` (списки `F64`), `Name` с полем `name`, `Camera` и `DirectionalLight` с пустыми `fields`. Названия полей `Transform`, `Name` и пустые поля `Camera`/`DirectionalLight` **не проверены** по реализации `#[derive(Reflect)]` (шаг 1). Шаблоны в `project_template.rs` надо открыть `SceneDocument::open` и проверить.
10. `EntityGuid` в тексте шаблонов записан строкой из 32 hex-символов (формат не проверен, шаг 2).
11. `TypeRegistry::snapshot_world(&World) -> WorldSnapshot`, `TypeRegistry::new`, `register_engine_defaults(&mut TypeRegistry)` — проверено по коду; `WorldSnapshot: PartialEq + Clone + Debug` — проверено.
12. `World::new`, `spawn`, `add_component`, `get_component_mut` — проверено.
13. `UndoStack: Default`, `Selection::clear` — проверено (`undo.rs`, `selection.rs`).
14. `ron 0.12`: `ron::ser::to_string_pretty(&T, PrettyConfig::default())`, `ron::from_str::<T>` — тот же вызов, что в `redox_editor_ui/src/dock.rs`.
15. egui 0.36: `Window::open(&mut bool)`, `ui.radio_value`, `ui.colored_label(Color32, &str)`, `ui.add_enabled(bool, Button)`, `egui::Align2::CENTER_CENTER` — по образцу `command_palette.rs`, сигналы не сверены с исходником 0.36.
16. winit: `Window::set_title(&self, &str)`, `ActiveEventLoop::exit()` — как в существующем коде (`elwt.exit()`).

Конфликты слияния, которые вероятны:
- `crates/redox_editor_scene/Cargo.toml` и `src/lib.rs` (шаг 3 тоже добавит `redox_reflect` и `mod`-строки). Оставить по одной строке каждого.
- `crates/redox_editor_scene/src/lib.rs` (реэкспорты `SceneDocument` и других).

## СПИСОК ДЛЯ КООРДИНАТОРА

Команды по порядку (после слияния шагов 1–3, иначе будут ошибки «unresolved import» в `level.rs` и `file_menu.rs`):
1. `cargo fmt --all` (возможны переносы длинных строк в `project_template.rs` и `file_menu.rs`, например строка с `ui.label("Type a path: ...")`).
2. `cargo check --workspace --all-targets`.
3. `cargo clippy --workspace --lib --bins --tests -- -D warnings`. Ожидаемые места: `file_menu.rs` (не переписать `if let`/`let else`), `project.rs` (`if path.is_dir() { if !is_private {…} }` может потребовать collapsible-if).
4. `cargo test -p redox_editor_core` — ожидается: прежние 16 `undo` и 7 из `tests/context.rs`, плюс 12 из `tests/project.rs`, плюс 5 из `file_state`, плюс 2 из `file_commands`.
5. `cargo test -p redox_editor_scene --test level` — 4 теста. Если упадёт `scene_file_opens_as_a_scene_level` с ошибкой Parse, это проверка шаблона/формата из шага 3 (см. пункт 9 предположений).
6. `cargo test -p redox_editor` — `crate_layout` (4 теста), в том числе проверка, что `redox_editor_core` не зависит от других редакторских крейтов (ни `redox_editor_scene`, ни `redox_editor_ui`).
7. `cargo tree -p redox_editor_core -e normal | findstr egui` — должно быть пусто.
8. `python scripts/check_file_sizes.py` и `python scripts/gen_crates_doc.py --check`. Новые зависимости: `redox_editor_core` (serde, ron, thiserror), `redox_editor_scene` (redox_reflect, thiserror), возможно `docs/CRATES.md` нужно перегенерировать.
9. Проверка шаблонов: добавить (или запустить) тест «`ProjectTemplate::ThreeDStarter.scene_text()` открывается `SceneDocument::open` без предупреждений». Ошибки здесь указывают на поля `Transform`/`Name` или формат `EntityGuid` (предположения 9 и 10).
10. Ручной дым-тест (`cargo run -p redox_editor`): File > New Project в пустой папке с шаблоном «3D starter» → проект создан, сцена открыта; изменить объект → заголовок `untitled *`/`main *`; Ctrl+S → заголовок без звёздочки; закрыть окно при несохранённых изменениях → появляется «Unsaved changes»; Cancel оставляет окно; Save/Don't save закрывает. Ctrl+Shift+S сохраняет layout (файл `editor_layout.ron` обновляется). File > Recent Projects содержит проект.
11. Проверить `editor_recent.ron` рядом с `editor_layout.ron` и `.redox/.gitignore` в созданном проекте.

## Не сделано / отклонения

- Шаги 1–3 (Reflect, `EntityGuid`, `SceneDocument`, `LoadReport`, `*.scene.ron`) в этой ветке не делались: они у параллельных агентов. Поэтому ветка сейчас **не компилируется** до слияния (ожидаемо).
- Сохранение мира (`World`-уровень) пишет только `world.ron`. Актёры пишутся операциями кисти. Общего «сохранить все актёры» нет; при изменении мира без кисти изменения не сохраняются, а `is_dirty` их видит. Требуется решение координатора: либо закрыть это в A2 отдельно, либо оставить до A3/A6.
- `WorldSession` в `EditorUiCtx` (старая связь с миром) не синхронизируется с `FileMenu::level`: открытие уровня-мира через File не меняет `world_session`. Сделано так, чтобы не трогать `EditorUiCtx` (общий файл).
- Шаблон `3d_starter` без `MeshHandle` (предположение 9).
- Тесты UI (окна egui) не писались: по спеке достаточно тестов состояний в `file_state` и ручного дыма (пункт 10).
- Тест «`ProjectTemplate` открывается `SceneDocument`» в этом шаге не добавлен: тип `SceneDocument` ещё не существует в ветке. Добавить после слияния (пункт 9).
- Пункт спеки «родной диалог выбора файла» не делался по правилу спеки (в A2 путь вводится текстом).
- Правило «не запускать»: ничего не запускалось, поэтому все проверки выше — за координатором.

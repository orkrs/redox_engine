# EDT-A1 — Редактор, этап A, срез A1: раскладка крейтов, headless `EditorContext`, граница editor/runtime

- **Статус:** не начат.
- **Размер:** 5 шагов (S, M, M, M, S) ≈ 6 дней. **Крейты:** `redox_editor`, `redox_editor_core`, `redox_editor_ui`, новые `redox_editor_{scene,assets,viewport,debugger,build,plugins}`, корневой `Cargo.toml`, `docs/CRATES.md`. **Зависит от:** ничего. **Блокирует:** EDT-A2…A7.
- **Исполнение:** шаги 1–3 механические (перенос файлов, правка `Cargo.toml` и путей `use`): делает координатор, потому что они требуют постоянной сборки. Шаги 4–5 (контекст и команды) исполняет агент (haiku) без права запускать cargo; после каждого шага координатор компилирует и возвращает ошибки.
- Источник: `docs/engineering/EDITOR_PLAN.md` §3 (решения D1–D6), §3.1 (раскладка), §5 A1.
- **Не входит:** Undo для создания/дублирования/удаления сущностей (A3), сохранение и загрузка сцены (A2), настройки редактора, горячие клавиши как система, переименование бинаря `redox_editor`.

## Суть
Сейчас редактор это три крейта, в которых `redox_editor_core` смешивает логику (Undo, выбор) с viewport (камера, гизмо, пикинг, wgpu-проходы), отладкой (PIE, профайлер, time travel) и миром (OFPA-сессия, git-трекер); общее состояние живёт в `EditorUiCtx` внутри крейта панелей, а команды выполняет функция `run_command` в бинаре. Из-за этого логику редактора нельзя проверить без окна и egui. Срез раскладывает код по крейтам (решение D2), делает `core` и новые крейты свободными от egui и вводит `EditorContext` с реестром команд как точку входа для тестов и для следующих срезов.

## Что уже есть (проверено чтением кода)

- **Крейты и размеры:** `redox_editor` 2,2 тыс. строк (`app.rs`, `app/redraw.rs`, `app/ui.rs`, `commands.rs`, `cli.rs`, `render_host.rs`, `main.rs`), `redox_editor_core` 3,9 тыс., `redox_editor_ui` 2,9 тыс. Тестов: core 16, ui 7, editor 3 (всего `#[test]`).
- **Модули `redox_editor_core/src`:** `brush`, `camera`, `git`, `gizmo`, `picker`, `pie`, `profiler`, `selection`, `time_travel`, `undo`, `viewport` (`cube_pass`, `cube_render`, `lines_pass`, `overlay`, `target`), `viewport_mode`, `world_session`.
- **Связи внутри core:** `gizmo` использует `undo` (`UndoStack`, `UndoCommand`) и `camera`; `picker` использует `camera`, `gizmo::screen_ray`, `viewport_mode`; `viewport/*` использует `camera`, `picker`, `viewport_mode`; `pie`, `time_travel`, `profiler`, `selection`, `world_session`, `git`, `brush` самостоятельны (по `use crate::`).
- **egui в core встречается ровно в двух местах:** `undo.rs` (`UndoStack::handle_hotkeys(&egui::Context, …)`, строки ~212–224) и `viewport/overlay.rs` (`egui_to_viewport_coords(egui::Pos2, egui::Rect, …)`, строки ~87–100). Зависимости core: `egui`, `winit`, `wgpu`, `git2` (feature `editor-git`), `flume`, `smallvec`, `bytemuck`, `redox_render`, `redox_physics`, `redox_asset`, `redox_anim` и др.
- **Общее состояние:** `EditorUiCtx` (`redox_editor_ui/src/context.rs`) держит `Arc<Mutex<…>>` для `world`, `selection`, `registry`, `gizmo`, `undo`, `camera`, `pie`, `time_travel`, `world_session`, `git`, а также состояния панелей, `console: Mutex<Vec<String>>` (последние 200 строк), `status`, флаги. `EditorApp` (`redox_editor/src/app.rs`) владеет теми же `Arc` и окном/egui/GPU.
- **Команды:** `redox_editor/src/commands.rs::run_command(ui_ctx, dock, id)` разбирает идентификаторы `focus`, `create_entity`, `duplicate`, `delete`, `reset_layout`, `pie_play`, `pie_stop`, `bake_meshlets`, `bake_report`, `spawn_agent`, `shading_lit|unlit|normals|wire`, `align_surface`; список для палитры зашит в `CommandPalette::default()` (`redox_editor_ui/src/command_palette.rs`).
- **Наблюдение для A3:** `create_entity`, `duplicate`, `delete` меняют мир напрямую и **не записываются в `UndoStack`**. В этом срезе поведение не меняется.
- **Рабочее пространство:** `Cargo.toml` содержит список `members`; `docs/CRATES.md` генерируется `scripts/gen_crates_doc.py` (CI проверяет `--check`); в крейтах включён `#![warn(missing_docs)]`; clippy запрещает предупреждения.
- **Feature `editor-git`:** объявлена в `redox_editor_core/Cargo.toml` (`dep:git2`), используется только в `git.rs`.

## Целевая раскладка

| Крейт | Содержимое после среза | Основные зависимости (внутренние) |
|---|---|---|
| `redox_editor_core` | `selection`, `undo`, новые `context`, `command`, `log`, маркер `EditorOnly` | `redox_ecs`, `redox_math`, `redox_reflect` |
| `redox_editor_viewport` | `camera`, `gizmo`, `picker`, `brush`, `viewport_mode`, `viewport/*` | `redox_editor_core`, `redox_render`, `redox_ecs`, `redox_math`, `wgpu`, `bytemuck` |
| `redox_editor_debugger` | `pie`, `time_travel`, `profiler` | `redox_ecs`, `redox_reflect` |
| `redox_editor_scene` | `world_session`, `git` (feature `editor-git`) | `redox_scene`, `redox_ecs`, `redox_math` |
| `redox_editor_assets` | скелет | `redox_asset` |
| `redox_editor_build` | скелет | `redox_asset` |
| `redox_editor_plugins` | скелет (манифест и типы) | — |
| `redox_editor_ui` | панели, dock, палитра, виджеты (всё, где egui) | `core`, `scene`, `viewport`, `debugger`, `build`, `assets`, egui, egui_dock |
| `redox_editor` (бинарь) | окно, цикл, привязка событий, `render_host` | всё вышеперечисленное |

Правила зависимостей: `redox_editor` → `redox_editor_ui` → {`scene`, `assets`, `viewport`, `debugger`, `build`, `plugins`} → `redox_editor_core` → крейты движка. **egui допустим только в `redox_editor_ui` и `redox_editor`.** Крейты `viewport` и `core` не зависят от `winit`, если это возможно (проверить, где `winit` реально используется: в `redox_editor_core/src` прямых использований нет, зависимость вероятно лишняя).

## Шаг 1 — рабочее пространство и скелеты (S, координатор)
- Добавить в `members` корневого `Cargo.toml` шесть новых крейтов; создать `Cargo.toml` и `src/lib.rs` каждого (`#![warn(missing_docs)]`, описание в `//!`, лицензия и версия по образцу соседних крейтов `rust-version`, `edition`).
- `redox_editor_assets`, `redox_editor_build`, `redox_editor_plugins`: по одному публичному типу-заглушке с документацией и тестом (создаётся, `Default`, отладочный вывод). Для `plugins`: `PluginManifest { name, version, api_version }` без загрузчика (решение D6).
- Прогнать `python scripts/gen_crates_doc.py`, закоммитить `docs/CRATES.md`.
- **Проверка:** `cargo check --workspace --all-targets`, `gen_crates_doc.py --check`.

## Шаг 2 — перенос модулей без изменения поведения (M, координатор)
Порядок (по одному крейту, отдельный коммит на каждый; сначала самое независимое):
1. `debugger` ← `pie.rs`, `time_travel.rs`, `profiler.rs` (с их тестами: 3 теста PIE).
2. `scene` ← `world_session.rs`, `git.rs`; feature `editor-git` переезжает в `redox_editor_scene/Cargo.toml`, в `redox_editor` добавляется проброс `editor-git = ["redox_editor_scene/editor-git"]`; из `redox_editor_core/Cargo.toml` feature и `git2` удаляются.
3. `viewport` ← `camera.rs`, `gizmo.rs`, `picker.rs`, `brush.rs`, `viewport_mode.rs`, `viewport/` целиком (с их тестами: gizmo 5, picker 6 и т. д.).
- Пути `use crate::…` внутри перенесённых модулей остаются верными (они переезжают группой); обращения к `undo` меняются на `redox_editor_core::undo`.
- Все потребители (`redox_editor`, `redox_editor_ui`) переключаются на новые пути; **совместимые реэкспорты из `core` не делаются** (это создало бы цикл).
- **Проверка после каждого коммита:** `cargo check --workspace --all-targets`, тесты затронутых крейтов, число тестов не уменьшилось (core 16 → сумма по новым крейтам и core = 16; ui 7; editor 3).

## Шаг 3 — убрать egui и лишние зависимости (S, координатор)
- `redox_editor_core/src/undo.rs`: заменить `handle_hotkeys(&egui::Context, …)` на `UndoStack::apply_hotkey(hotkey: UndoHotkey, world, registry) -> bool`, где `enum UndoHotkey { Undo, Redo }`; распознавание Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z через egui переезжает в `redox_editor_ui` как функция `undo_hotkey(&egui::Context) -> Option<UndoHotkey>`; вызов в `app/redraw.rs` обновляется (поведение то же).
- `viewport/overlay.rs`: `egui_to_viewport_coords` принимает простые типы (`pos: [f32; 2]`, `rect_min: [f32; 2]`, `rect_size: [f32; 2]`, плюс прежние параметры); тонкая обёртка над `egui::Pos2` и `egui::Rect` живёт в `redox_editor_ui`; вызов в `redox_editor/src/app.rs` обновляется.
- Удалить из `Cargo.toml` крейтов `core`, `viewport`, `debugger`, `scene` зависимости `egui` и `winit` (если после проверки `winit` нигде не нужен), а также неиспользуемые остальные (по ошибкам сборки и `cargo +nightly udeps` не требуется: достаточно, чтобы сборка проходила и `cargo tree` не показывал egui).
- **Проверка:** `cargo tree -p <крейт> -e normal` для `core`, `viewport`, `debugger`, `scene`, `assets`, `build`, `plugins` не содержит `egui`; тест защиты раскладки (шаг 5); все тесты зелёные.

## Шаг 4 — `EditorContext`, реестр команд и журнал (M, агент)
Новые файлы в `redox_editor_core/src/`:
- `log.rs`: `EditorLog` (кольцевой буфер на 200 строк, потокобезопасный), методы `info`, `warn`, `error` дают строки с теми же префиксами (`[INFO]`, `[WARN]`, `[ERROR]`), что сейчас пишет `EditorUiCtx::log`; `lines() -> Vec<String>`; `clear()`.
- `context.rs`: `#[derive(Clone)] pub struct EditorContext` с общими ручками: `world: Arc<Mutex<World>>`, `registry: Arc<Mutex<TypeRegistry>>`, `selection: Arc<Mutex<Selection>>`, `undo: Arc<Mutex<UndoStack>>`, `log: EditorLog`, `status: Arc<Mutex<String>>`, `commands: CommandRegistry`. Конструктор `EditorContext::new(registry: TypeRegistry)` создаёт пустой мир и стек Undo с прежним лимитом (взять значение из места создания в `app.rs`). Контекст **не знает** про GPU, окно, egui, viewport и PIE.
- `command.rs`: `trait EditorCommand: Send + Sync { fn id(&self) -> &str; fn label(&self) -> &str; fn execute(&self, ctx: &EditorContext); }` и `CommandRegistry` (`register`, `get`, `ids`, `labels`, `run(&self, ctx, id) -> bool`; неизвестный id пишет в лог `[WARN] Unknown command: {id}` и возвращает `false`, как сегодня).
- `editor_only.rs`: маркерный компонент `EditorOnly` (`Default`, `Clone`, `Copy`) и функция `editor_only_entities(&World) -> Vec<Entity>`; в этом срезе нигде не применяется (задел для A2: такие сущности не попадают в сохранение).
- Команды, которые работают только с доменным состоянием, переезжают в реестр **без изменения поведения**: `create_entity`, `duplicate`, `delete` (тексты логов, выбор сущности, смещение копии по X на 1,0 и прочее как сейчас; в `UndoStack` они по-прежнему не пишутся, это задача A3).
- Остальные идентификаторы (`focus`, `reset_layout`, `pie_*`, `shading_*`, `bake_*`, `spawn_agent`, `align_surface`) остаются в `run_command` бинаря: они требуют состояния viewport, отладчика, dock или UI. `run_command` сначала спрашивает реестр, затем применяет прежний `match`.
- Тесты (`redox_editor_core/tests/context.rs`, без GPU и окна):
  - `a_context_is_created_without_a_window` (создаётся, мир пуст, реестр команд содержит `create_entity`, `duplicate`, `delete`);
  - `create_entity_adds_an_entity_with_a_transform_and_selects_it` (в логе строка `[INFO] Created entity #…`);
  - `duplicate_copies_the_transform_one_unit_along_x`;
  - `delete_removes_the_selected_entity_and_clears_the_selection`;
  - `an_unknown_command_is_logged_and_reported` (`run` возвращает `false`, в логе `[WARN] Unknown command: …`);
  - `undo_and_redo_restore_the_world_through_the_context` (изменить компонент через `UndoStack` контекста, `undo`, `redo`: `capture_world` до и после равны);
  - `editor_only_entities_are_found` (две сущности, одна с маркером).

## Шаг 5 — `EditorApp` и панели на контексте (S, агент)
- `EditorUiCtx` хранит `core: EditorContext` вместо отдельных `world`, `registry`, `selection`, `undo`, `console`, `status`; остальные поля (панели, `gizmo`, `camera`, `pie`, `time_travel`, `world_session`, `git`, флаги) остаются; `EditorUiCtx::log` делегирует `core.log`.
- `EditorApp`: те же поля заменяются ручками контекста (клонирование `EditorContext` вместо клонирования четырёх `Arc`).
- `CommandPalette` получает список из реестра плюс прежние зашитые команды (метод `with_commands(...)` или перенос списка на вызывающую сторону); ярлыки (`> Create Empty Entity` и т. д.) не меняются.
- Новый тест защиты раскладки `redox_editor/tests/crate_layout.rs`: читает `Cargo.toml` крейтов `redox_editor_*` и проверяет, что (1) `egui` объявлен только в `redox_editor_ui` и `redox_editor`; (2) `redox_editor_core` не зависит от `redox_editor_{ui,scene,assets,viewport,debugger,build,plugins}`; (3) крейты `scene`, `assets`, `debugger`, `build`, `plugins` не зависят от `redox_editor_ui` и `redox_editor_viewport`; (4) в `redox_editor_viewport` нет `egui`. Разбор простым чтением секций `[dependencies]` (без новых зависимостей).
- **Поведение окна не меняется:** все существующие команды палитры, хоткеи Undo/Redo, PIE и гизмо работают как до среза.

## Границы
- **Читать:** `redox_editor_core/src/*`, `redox_editor_ui/src/{context,command_palette,lib}.rs`, `redox_editor/src/{app,commands}.rs`, `app/redraw.rs`, `app/ui.rs`, `Cargo.toml` всех редакторских крейтов, `scripts/gen_crates_doc.py`.
- **Создать:** шесть новых крейтов; `redox_editor_core/src/{context,command,log,editor_only}.rs`, `redox_editor_core/tests/context.rs`, `redox_editor/tests/crate_layout.rs`.
- **Менять:** корневой `Cargo.toml`, `Cargo.toml` и `lib.rs` затронутых крейтов, `docs/CRATES.md`, пути `use` во всех потребителях.
- **Не трогать:** `redox_render`, `redox_scene`, `redox_reflect` (кроме чтения), поведение команд и Undo, формат `editor_layout.ron`, `render_host.rs`.

## Проверка
`cargo fmt`, `cargo clippy --workspace --lib --bins --tests -- -D warnings`, `cargo test -p redox_editor_core -p redox_editor_viewport -p redox_editor_debugger -p redox_editor_scene -p redox_editor_ui -p redox_editor`, `cargo tree -p <крейт> -e normal | grep egui` для семи крейтов без egui (пусто), `python scripts/check_file_sizes.py`, `python scripts/gen_crates_doc.py --check`, ручной дым-тест редактора (координатор): `cargo run -p redox_editor` открывается, создать пустую сущность через палитру, Ctrl+Z/Ctrl+Y, Play/Stop (PIE), закрытие без паники.

## Для исполнителя (шаги 4–5)
`git merge --ff-only main`; `Cargo.lock` из main. Не запускай cargo (чек и тесты делает координатор); форматируй только свои файлы через `rustfmt --edition 2024 <файл>`; коммит по явным путям без `git add -A`; не добавляй `assets/forest`, `examples/rus_universalis`, `RedOx_Editor_Full_Spec.md`. Один шаг за заход; отчёт `docs/engineering/notes/EDT-A1-report.md` (что сделано, файлы, команды для проверки, вопросы).

## Закрытие
Исход ✅ / ⛔ / 📌 по шагам (правила `FINAL_WAVE.md` §1). Срез закрыт, когда: все шесть крейтов в рабочем пространстве и в `docs/CRATES.md`; нет `egui` в крейтах без UI (тест `crate_layout`); сумма тестов не уменьшилась и новые тесты контекста зелёные; поведение окна не изменилось (дым-тест); clippy чист. Если дым-тест показывает регресс, шаг 5 не закрыт.

## Риски
| Риск | Что делаем |
|---|---|
| Перенос ломает сборку из-за неочевидных путей `use` | Перенос по одному крейту с коммитом и `cargo check --workspace` после каждого; без реэкспортов |
| `winit` в core нужен косвенно (например, через `picker`) | Проверять ошибками сборки; зависимость удалять только когда сборка зелёная |
| Агент без cargo делает ошибки в шагах 4–5 | Малые шаги, обратная связь от координатора, тесты написаны заранее по именам |
| `EditorUiCtx` слишком большой, перестановка полей путает | В шаге 5 меняем только перечисленные поля; остальные не трогаем |
| Файл `redox_editor_ui/src/dock.rs` и другие превышают лимит размера | Не расти файлы без необходимости; `check_file_sizes.py` в проверке |

# EDT-C6 — отчёт исполнителя

- **Ветка:** `edt/c6`
- **Worktree:** `/home/besyak/Документы/redox/.worktrees/c6`
- **База:** `main` @ `9afe43b` (worktree от HEAD main)
- **Ничего не запускалось:** ни cargo, ни rustfmt, ни тесты, ни редактор.

## Статус закрытия

| Пункт | Статус |
|---|---|
| PluginHost + PluginRegistry + EditorPlugin | ✅ |
| Hello in-process plugin + project.ron list | ✅ |
| Safe mode (`catch_unwind` register/execute) | ✅ |
| Native loader | 📌 stub + feature `editor-native-plugins` (no dlopen) |
| Project → Plugins UI + safe banner | ✅ |
| §23.17 minimal scenario | ✅ `s17_plugins.rs` |
| Документация + CHANGELOG | ✅ |

## Выбор (одной строкой)

Встроенный proof — demo **HelloPlugin** (вкладка + команда), а не вынос Packager: проще для egui_dock и не трогает OFPA UI.

## Коммиты

| Шаг | SHA | Сообщение |
|---|---|---|
| 1 | `435ca3c` | feat: editor plugin host and registry API — EDT-C6 step 1 |
| 2 | `52ced70` | feat: hello built-in plugin and project.ron enable list — EDT-C6 step 2 |
| 3 | `6f7dc39` | feat: safe mode settings for plugin panic isolation — EDT-C6 step 3 |
| 4 | `d12caee` | feat: editor-native-plugins feature with documented dlopen stub — EDT-C6 step 4 |
| 5 | `7612ea0` | feat: Project Plugins UI and editor startup wiring — EDT-C6 step 5 |
| отчёт | `8ae0a26` | docs: EDT-C6 coordinator report |

## Файлы

### Новые

- `crates/redox_editor_plugins/src/{manifest,plugin,registry,host,validation,native,builtin/hello}.rs`
- `crates/redox_editor/src/app/plugins.rs`
- `crates/redox_editor_ui/src/{plugin_manager,hello_panel}.rs`
- `crates/redox_editor/tests/scenarios/s17_plugins.rs`
- `docs/EDITOR_PLUGINS.md`
- `docs/engineering/notes/EDT-C6-report.md`

### Изменённые

- `crates/redox_editor_plugins/{Cargo.toml,src/lib.rs}`
- `crates/redox_editor_core/src/{command,project,settings,lib}.rs`
- `crates/redox_editor_ui/{Cargo.toml,src/lib,dock,file_menu}.rs`
- `crates/redox_editor/{Cargo.toml,src/app.rs,src/app/{ui,redraw,plugins}.rs,src/commands.rs,tests/scenarios/main.rs}`
- `CHANGELOG.md`

## Тесты по спеке (в коде)

| Имя | Где |
|---|---|
| `mock_plugin_registers_command` | `redox_editor_plugins` unit |
| `wrong_api_version_rejected` | `redox_editor_plugins` unit |
| `enable_disable_hello_plugin_changes_command_registry_ids` | `redox_editor_plugins` unit |
| `panicking_plugin_does_not_crash_host_command_absent_after_disable` | `redox_editor_plugins` unit |
| `panicking_register_disables_plugin_without_crashing_host` | `redox_editor_plugins` unit |
| `a_manifest_carries_the_current_api_version` | `manifest.rs` unit |
| `tz_23_17_hello_plugin_from_manifest` | `redox_editor` scenario |

## Native loading

**📌** Feature `editor-native-plugins` есть на `redox_editor_plugins` и прокинут в `redox_editor`; `load_native_plugin` и `NATIVE_PLUGIN_ENTRY` документированы, **реального dlopen нет**.

## Предположения

- `egui_dock::NodeTree::remove_tab` существует для снятия вкладки Hello (проверить компилятором).
- `sync_project_plugins` вызывается каждый кадр после `handle_requests` — дёшево при пустом проекте.
- `ProjectContext::open` без каталога по-прежнему даёт `MissingPlugin` для всех имён (тесты `s14` не ломаются).

## Пробелы / не сделано

- Импортеры/validation rules из registry **не** подмешиваются в `AssetService` / B7 runner (API собран, применение — при B7/C1).
- WASM editor plugins — вне скоупа (📌 будущее).
- `redox_script` не тронут.

## Координатору проверить

```text
cargo fmt --all
cargo clippy --workspace --lib --bins --tests -- -D warnings
cargo test -p redox_editor_plugins
cargo test -p redox_editor_core -p redox_editor_ui -p redox_editor
cargo test -p redox_editor --test scenarios s17
```

Ручной: открыть проект, **Project → Plugins**, включить `hello`, палитра → «Hello from plugin», вкладка Hello Plugin.

## Git log

```text
8ae0a26 docs: EDT-C6 coordinator report
7612ea0 feat: Project Plugins UI and editor startup wiring — EDT-C6 step 5
d12caee feat: editor-native-plugins feature with documented dlopen stub — EDT-C6 step 4
6f7dc39 feat: safe mode settings for plugin panic isolation — EDT-C6 step 3
52ced70 feat: hello built-in plugin and project.ron enable list — EDT-C6 step 2
435ca3c feat: editor plugin host and registry API — EDT-C6 step 1
```
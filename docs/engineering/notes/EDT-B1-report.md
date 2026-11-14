# EDT-B1 — Outliner и иерархия (отчёт исполнителя)

**Статус:** ✅ сдано на координацию (сборку/тесты не запускал).  
**Ветка:** `edt/b1`  
**Worktree:** `/home/besyak/Документы/redox/.worktrees/b1`

## Коммиты

Координатор: после `git log edt/b1 --not main` заполнить хеши. Ожидаются сообщения вида `feat: … — EDT-B1 step N` (шаги 1–5).

## Что сделано (по шагам)

| Шаг | Содержание |
|-----|------------|
| 1 | `outliner_model`: узлы, сборка из `World` + `GuidIndex`, фильтры, циклы → корень + warning; unit-тесты A→B→C, фильтры, цикл |
| 2 | `outliner_ops::reparent` + Undo; тесты keep_world, undo, цикл потомка |
| 3 | Мультивыбор (range/children/invert), hide/lock/isolate + Undo; `Visibility`, `Locked`, `EditorFolder` |
| 4 | `visible_row_range`, виртуальный список в UI; `#[ignore] bench_build_50k_entities`; каркас в `docs/BENCHMARKS.md` |
| 5 | Панель: scene tree vs OFPA по `session.is_none()`; контекстное меню; `s19_outliner_hidden_persist` |

## Файлы

**Созданы**

- `crates/redox_editor_scene/src/outliner_model.rs`
- `crates/redox_editor_scene/src/outliner_ops.rs`
- `crates/redox_editor_core/src/markers.rs`
- `crates/redox_editor_ui/src/outliner/mod.rs`, `world.rs`, `scene.rs`
- `crates/redox_editor/tests/scenarios/s19_outliner_hidden_persist.rs`
- `docs/engineering/specs/EDT-B1.md` (копия из main)
- `docs/engineering/notes/EDT-B1-report.md`

**Изменены**

- `crates/redox_editor_core/src/selection.rs`, `lib.rs`
- `crates/redox_editor_scene/src/lib.rs`, `register.rs`
- `crates/redox_editor_ui/src/dock.rs`, `context.rs` (без логики)
- `crates/redox_editor/src/app/redraw.rs` (`FocusScene`)
- `crates/redox_editor/tests/scenarios/main.rs`
- `docs/BENCHMARKS.md` (секция EDT-B1)

**Удалён**

- `crates/redox_editor_ui/src/outliner.rs` (заменён модулем `outliner/`)

## Допущения

- Режим сцены: `WorldSession` в UI **не** открыт (`scene_mode = session.is_none()`), как у плоского списка в A7; OFPA без изменений при открытом world session.
- `validation_errors` на панели пока пустой ввод (B7 позже); фильтр «errors» готов к подключению.
- Rename в контекстном меню ведёт в Inspector / статусную строку (полноценный in-place rename не входил в B1).
- «Reparent to…» реализован как drop на узел + «Reparent to root»; отдельного диалога нет (простейший совместимый вариант из спеки).

## Пробелы / риски

- Виртуальный scroll берёт `scroll_y` из геометрии egui; координатору стоит проверить корректность на больших сценах и подкрутить при необходимости.
- Порог 50k в `BENCHMARKS.md` не заполнен (ждёт координатора).
- Drag-and-drop reparent не блокирует drop на потомка (ошибка придёт из `UndoCommand::reparent` + WARN в лог).

## Координатору проверить

```text
cargo fmt
cargo clippy --workspace --lib --bins --tests -- -D warnings
cargo test -p redox_editor_scene -p redox_editor_core -p redox_editor_ui -p redox_editor
cargo test -p redox_editor_scene outliner_model::tests::bench_build_50k_entities -- --ignored --nocapture
```

Ожидание: все перечисленные тесты `outliner_model` зелёные; сценарий `s19_outliner_hidden_persist`; OFPA-ветка outliner по-прежнему работает при `world_session = Some`.

## Закрытие

✅ Дерево, reparent+Undo, hide/lock/isolate, мультивыбор, виртуальный список; OFPA-режим сохранён; тесты написаны (прогон — координатор).
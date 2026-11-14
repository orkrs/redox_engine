# EDT-B5 — отчёт исполнителя

**Статус:** ✅ (код и тесты сданы; прогоны — координатор).  
**Ветка:** `edt/b5`  
**Worktree:** `/home/besyak/Документы/redox/.worktrees/b5`

## Выбор
- Undo для жестов графа: **снимки всего `MaterialGraph` на commit** (`MatGraphUndo`), без интеграции в `UndoStack` мира (A3).
- Превью освещение: **B3 не подключён** → `PreviewLighting::Simple` + `SimplePreviewLight::studio_fallback()`.
- Per-instance overrides: **📌 только asset-level `Param` nodes** (тип в движке есть, UI инстанса не делался).

## Шаги

| Шаг | Содержание | Тесты |
|-----|------------|-------|
| 1 | `MaterialDocument`, I/O, dirty, migrate, save после validate+emit | `redox_editor_assets/tests/material_document.rs` |
| 2 | `MatGraphView`, connect, undo snapshots | `redox_editor_assets/tests/mat_graph_view.rs` |
| 3 | Material mode на `NodeCanvas`, `CompileReport` + focus node | `material_compile.rs`, UI в `dock` + `material_panel` |
| 4 | `MaterialPreviewSettings`, простой свет, `preview_revision` | `redox_editor/tests/material_preview_gpu.rs` (`#[ignore]`) |
| 5 | Панель параметров, read-only WGSL | `redox_editor_ui/tests/material_session.rs` |
| 6 | `AssetKind::Material`, Open/dbl-click, палитра, s07 | `browser_matgraph.rs`, `s07_material_editor.rs` |

## Правило ТЗ
`MaterialDocument::save` / `write_to` вызывают `compile_for_save` до `write_atomic`; при ошибке файл не трогается (тесты `invalid_graph_save_err_and_disk_unchanged`, `s07_invalid_matgraph_is_not_written`).

## Координатору проверить
```bash
cd /home/besyak/Документы/redox/.worktrees/b5
cargo fmt
cargo clippy --workspace --lib --bins --tests -- -D warnings
cargo test -p redox_editor_assets --test material_document --test mat_graph_view --test material_compile --test browser_matgraph
cargo test -p redox_editor_ui --test material_session
cargo test -p redox_editor --test scenarios s07_invalid_matgraph_is_not_written
cargo test -p redox_editor --test material_preview_gpu -- --ignored
```

## Gaps
- Viewport не рисует отдельную сферу превью (настройки mesh/light в сессии; GPU-тест отдельным бинарником).
- Hot reload в viewport: `preview_revision` для хоста; полная связка с `MatGraphWatcher` в UI — след. итерация.
- Добавление/удаление узлов через UI не подключено (модель `MatGraphView` + API есть).
- Instance overrides материала на сущностях — 📌.

## Файлы (новые/существенные)
- `crates/redox_editor_assets/src/material_document.rs`, `mat_graph_view.rs`, `material_compile.rs`, `material_preview.rs` + tests
- `crates/redox_editor_ui/src/material_editor.rs`, `material_canvas.rs`, `material_panel.rs` + `node_canvas` Material mode
- `crates/redox_editor/src/app/material.rs`
- `crates/redox_editor/tests/material_preview_gpu.rs`, `scenarios/s07_material_editor.rs`
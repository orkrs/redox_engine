# EDT-B8 — отчёт исполнителя

- **Ветка:** `edt/b8-c1`
- **Worktree:** `/home/besyak/Документы/redox/.worktrees/b8-c1`
- **validate API (для B7):** `redox_editor_build::validate_project(root: &Path) -> Result<ValidationReport, ProjectError>`

## Шаг 1 — `redox_cook validate`

- Команда `validate <project-root> [--json <out>]`, exit code из `ValidationReport::exit_code()`.
- JSON: `{ "issues": [ { "rule", "severity", "path", "guid", "message" } ] }`.
- Тесты: `crates/redox_cook/src/project_validate.rs`, `crates/redox_editor_build/src/tests.rs` (`broken_sample_reports_missing_asset`).

## Шаг 2 — `redox_cook import`

- Headless очередь через `AssetService` + `drain_import_events`.
- Тест: `crates/redox_cook/src/project_import.rs` (`import_reports_success_and_failure`).

## Шаг 3 — headless sample

- `redox_editor --validate-project <root>`, `--run-sample <root> [--frames N]` (кадры 📌 без GPU).
- Тест: `crates/redox_editor/src/headless.rs`.

## Шаг 4 — документация

- USAGE в `redox_cook/src/main.rs`, HELP в `redox_editor/src/cli.rs`, `CHANGELOG.md`, `PROJECT_MAP.md`.

## Выбор

- Образец `sample_project` в git без бинарных ассетов: тесты «чистого» проекта создают temp-проект (`ThreeDStarter`), broken — `sample_project_broken` из данных.

## Координатору проверить

```bash
cargo test -p redox_cook -p redox_editor -p redox_editor_build
cargo run -p redox_cook -- validate crates/redox_editor/tests/data/sample_project_broken --json /tmp/v.json
cargo run -p redox_editor -- --validate-project <path-to-installed-sample>
```

Ожидание: broken ≠ 0 и JSON с `MissingAsset`; temp/installed sample — 0.
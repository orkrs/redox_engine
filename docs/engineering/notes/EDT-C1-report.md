# EDT-C1 — отчёт исполнителя

- **Ветка:** `edt/b8-c1`
- **Worktree:** `/home/besyak/Документы/redox/.worktrees/b8-c1`

## Шаг 1 — профили

- `BuildProfile` / `BuildTarget` в `redox_editor_core::project`.
- `BuildService::list_profiles`, `resolve_active` в `redox_editor_build`.
- Тест: `manifest_roundtrip_with_two_profiles`.

## Шаг 2 — замыкание

- `CookPlan`, `InclusionReason`, `build_cook_plan` (сцена → handles → glTF image URI).
- Тест: `cook_plan_includes_shared_texture_and_skips_unused`.

## Шаг 3 — cook + editor-only

- `BuildSnapshot::from_world`, `BuildService::cook` → `.pak` + `cook_report.json` (атомарно).
- Тесты: `editor_only_entity_is_absent_from_build_snapshot`, `cook_report_lists_reason_per_guid`.

## Шаг 4 — GUI и CLI

- Packager: поля project root, **Cook project**.
- `redox_cook cook <project> [--profile] [--json]`.

## Шаг 5 — документы

- `CHANGELOG.md`, `KNOWN_LIMITS.md` (EDT-A-04), `PROJECT_MAP.md`, этот отчёт.

## Ограничения

- glTF зависимости: только URI в том же дереве assets; материал→текстура без явного URI не разворачивается.
- Пустой `build_profiles`: неявный `release` только в `resolve_profile_for_cook`, не в `resolve_profile`.

## Координатору проверить

```bash
cargo test -p redox_editor_build -p redox_cook
cargo run -p redox_cook -- cook <project> --profile release --json
```
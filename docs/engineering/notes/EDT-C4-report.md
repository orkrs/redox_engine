# EDT-C4 — отчёт исполнителя

Ветка: `edt/c4`. Worktree: `/home/besyak/Документы/redox/.worktrees/c4`. База: `9afe43b chore: pre-linux checkpoint main`.
**Ничего не запускалось**: ни cargo, ни workflow, ни Python-скрипты с `--check`. Проверки выполняет координатор.

## Коммиты

| Шаг | Сообщение | Файлы |
|---|---|---|
| 1 | `ci: pin engine job to rust-toolchain — EDT-C4 step 1` | `.github/workflows/ci.yml` |
| 2 | `ci: editor-pipeline job and helper script — EDT-C4 step 2` | `.github/workflows/ci.yml`, `scripts/ci_editor_pipeline.ps1` |
| 3 | `ci: golden-gpu workflow_dispatch and CI docs — EDT-C4 step 3` | `.github/workflows/ci.yml`, `scripts/ci_golden.ps1`, `CONTRIBUTING.md`, `PROJECT_MAP.md`, `CHANGELOG.md` |
| отчёт | `docs: EDT-C4 report` | этот файл |

## Jobs: включены vs gated

| Job | Статус на PR/push | Примечание |
|---|---|---|
| **engine** | **включён** | `dtolnay/rust-toolchain@1.96.0`, `Swatinem/rust-cache@v2`, fmt → clippy → `cargo test --workspace` → `check_file_sizes.py` → `gen_crates_doc.py --check`; timeout 120 min |
| **editor-pipeline** | **gated (`if: false`)** | Шаги готовы: `redox_cook validate/import/cook` на `crates/redox_editor/tests/data/sample_project`, assert non-zero на `sample_project_broken`, upload артефактов JSON. На HEAD нет CLI `validate`/`import`/`cook` (EDT-B8/C1) — координатор снимает `if: false` после merge |
| **golden-gpu** | **не required** | Только `workflow_dispatch` с input `run_golden: true`; вызывает `scripts/ci_golden.ps1` |
| **msrv**, **miri**, **docs**, **deny** | **включены** | Сохранены из шаблона; toolchain **engine**/**docs** переведены на 1.96.0 |

## Принятые решения

1. Runners: **windows-latest** для engine и editor (как в существующем шаблоне и CONTRIBUTING «Windows only»); **ubuntu-latest** только для `deny`.
2. Имя job `check` переименовано в **engine** по спеке; остальные jobs без переименования.
3. `editor-pipeline` дублирует логику в yaml и в `scripts/ci_editor_pipeline.ps1` (локальный/ручной прогон).
4. Golden: не `if: false`, а **workflow_dispatch** — соответствует спеке «optional» и не блокирует PR.
5. `cargo test --workspace` целиком (без nextest split); при timeout координатор фиксирует в issue / follow-up.

## Предположения

- Сигнатуры CLI после B8/C1: `redox_cook validate <root> [--json path]`, `import <root>`, `cook <root> --profile default [--json path]`; exit validate 0 / ≠0 как в EDT-B8.
- JSON-отчёты пишутся по путям, переданным в `--json` (в job — рядом с sample projects).
- `sample_project` содержит профиль `default` в `project.ron` после C1.

## Координатору проверить

```bash
# После merge ветки — первый прогон Actions на PR:
# engine, msrv, docs, deny должны стартовать; editor-pipeline skipped; golden-gpu не стартует.

# Локально (не на этом шаге агента):
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python scripts/check_file_sizes.py
python scripts/gen_crates_doc.py --check

# После EDT-B8/C1: в ci.yml у editor-pipeline заменить `if: false` на `if: true` (или удалить условие).
pwsh scripts/ci_editor_pipeline.ps1

# Golden (вручную в Actions или локально):
pwsh scripts/ci_golden.ps1
```

## Закрытие

📌 Workflow файлы готовы; первый зелёный non-GPU прогон и включение **editor-pipeline** — за координатором.
# Contributing

## Setup
- Rust: `rust-toolchain.toml` pins **1.96.0**; MSRV **1.95** is checked in the `msrv` CI job (`rust-version` in `Cargo.toml`).
- Windows is the only supported platform for now (see `docs/ROADMAP_STAGE_0_1.md`, section 1.6).

## Continuous integration (GitHub Actions)

Workflow: [`.github/workflows/ci.yml`](.github/workflows/ci.yml) (EDT-C4).

| Job | When it runs | What it checks |
|---|---|---|
| **engine** | Every push / PR | `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `scripts/check_file_sizes.py`, `scripts/gen_crates_doc.py --check` (toolchain from `rust-toolchain.toml`, `Swatinem/rust-cache`) |
| **editor-pipeline** | Disabled until EDT-B8/C1 merge (`if: false`) | `redox_cook validate` / `import` / `cook` on `sample_project` and broken validate on `sample_project_broken`; uploads `validate.json` / `cook_report.json` |
| **golden-gpu** | Manual only (`workflow_dispatch` + `run_golden`) | `scripts/ci_golden.ps1` (`skinned_demo --golden`); not required for PRs |
| **msrv**, **miri**, **docs**, **deny** | Same triggers as **engine** | MSRV check, Miri subset, `cargo doc`, `cargo-deny` |

Red X on **engine** (or **msrv** / **docs** / **deny**): fix formatting, clippy, tests, `docs/CRATES.md`, file-size limits, or dependency policy before merging.

## Before every commit
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python scripts/check_file_sizes.py
```
Rendering changes also need the golden frames:
```bash
cargo run --example skinned_demo -p redox_core -- --golden crates/redox_core/tests/golden
```
References are bit-exact only on the developer GPU; pass `--update-golden` after an *intentional* visual change.

## Style
- Files stay under 800 lines (`scripts/check_file_sizes.py`); split by responsibility.
- Every `unsafe` block carries a `// SAFETY:` comment (`clippy::undocumented_unsafe_blocks` is on in `redox_ecs`).
- No `unwrap()` on external input; return typed errors (`thiserror`) from library crates.
- Commits: `<type>: <description>` with types feat, fix, refactor, docs, test, chore, perf, ci.

## Architecture decisions
Record significant decisions in `docs/adr/` (copy the shape of an existing ADR).

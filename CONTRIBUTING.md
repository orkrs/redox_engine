# Contributing

## Setup
- Rust: `rust-toolchain.toml` pins stable; the minimum supported version is **1.88** (`rust-version` in `Cargo.toml`, checked in CI).
- Windows is the only supported platform for now (see `docs/ROADMAP_STAGE_0_1.md`, section 1.6).

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

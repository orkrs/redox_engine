# EDT-B7 report — scene validation

## Status

| Area | Marker |
|---|---|
| Rules engine + per-rule tests | ✅ |
| Fix / Select / Open | ✅ (Fix: default camera + unparent cycle) |
| Issues panel + hooks | ✅ |
| `validate_project` API | ✅ |
| Material compile rule | 📌 skipped (no headless B5 hook) |

## Branch / worktree

- Branch: `edt/b7`
- Worktree: `/home/besyak/Документы/redox/.worktrees/b7`

## Public API

```rust
pub fn validate_project(root: impl AsRef<Path>) -> Result<ValidationReport, ValidateProjectError>;
pub fn validate_level(
    world: &World,
    registry: &TypeRegistry,
    options: ValidateLevelOptions<'_>,
) -> ValidationReport;
```

## Commits

See `git log edt/b7` (messages: `feat: <why> — EDT-B7 step N`).

## Assumptions

1. **Material compile (rule 5):** No stable headless matgraph compile without GPU; rule omitted, documented as 📌.
2. **Texture mips:** Heuristic uses `.meta` `settings` text (`generate_mips`, `mip_levels`) and KTX2 header `levelCount`; large = max(w,h) > `ValidationSettings::large_texture_threshold` (default 2048).
3. **World bounds:** Default AABB ±1e6 around origin; override via `ValidationSettings::world_bounds`.
4. **Active camera:** Error when no `Camera` component; warning when components exist but `ActiveCamera` resource is unset.
5. **PIE / save hooks:** `ValidationSettings::{block_pie_on_error, warn_before_save}` default to `true`; wired in `commands.rs` / `file_menu.rs`.

## Gaps / follow-ups

- Rule 5 when B5 exposes headless compile.
- Project validation does not deduplicate issues across scenes.
- Per-frame validation only on level open and manual Re-validate (status `validation_refresh`).

## Coordinator — verify (no runs were done here)

```bash
cargo clippy -p redox_editor_scene -p redox_editor_ui -p redox_editor_assets -- -D warnings
cargo test -p redox_editor_scene --test validate_rules
cargo test -p redox_editor_ui validation_model
```

Expected: all tests pass; Issues tab in default dock; `validate_project` on sample project returns a report; broken asset path → `MissingAsset` with Select guid.
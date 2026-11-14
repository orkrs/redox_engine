# EDT-C2 report

- **Branch:** `edt/b4-c2`
- **Worktree:** `.worktrees/b4-c2`
- **Status:** ✅

## Steps

1. **Reference viewer** — `ReferenceViewerModel` / `ReferenceView` (incoming via `dependents`, outgoing from scene index). Test: `reference_viewer_lists_incoming` in `audit.rs`.
2. **Orphans & duplicates** — `find_orphans`, `find_duplicates` by `content_hash`. Tests in `audit.rs`.
3. **Size map & VRAM** — `build_size_map`, rough KTX2/PNG table, CSV/JSON export. Tests: `size_map_children_sum_parent`, `export_csv_roundtrip`.
4. **UI & CLI** — dock tabs `ReferenceViewer`, `AssetAudit`; `redox_cook audit <assets-dir> [--json]`.

## Assumptions

- Orphan = not referenced from any indexed scene and not in `AuditConfig::force_keep`.
- Outgoing asset refs limited to scene files (no glTF texture graph yet).
- VRAM `unknown` when format not in the rough table.

## Gaps

- None blocking; VRAM figures are estimates only (labelled in export).

## Coordinator checks

```bash
cargo test -p redox_editor_assets audit
cargo run -p redox_cook -- audit assets --json
cargo clippy -p redox_editor_assets -p redox_editor_ui -p redox_cook -- -D warnings
```
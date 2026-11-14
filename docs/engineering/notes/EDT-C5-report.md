# EDT-C5 — Source control for `*.scene.ron` (agent report)

- **Branch:** `edt/c5`
- **Worktree:** `/home/besyak/Документы/redox/.worktrees/c5`
- **Status:** ✅ (📌 distributed file locks — not in spec scope)

## Commits

| Step | Message (intended) |
|------|-------------------|
| 1 | `feat: scene git status for scene.ron — EDT-C5 step 1` |
| 2 | `feat: GUID-level scene diff — EDT-C5 step 2` |
| 3 | `feat: scene commit and ff-only pull — EDT-C5 step 3` |
| 4 | `feat: git overlay and scene status bar — EDT-C5 step 4` |

(Exact SHAs — see `git log` on `edt/c5` after coordinator commit review.)

## Files touched

### `redox_editor_scene`

- `src/scene_diff.rs` — `diff_scene_files`, `SceneEntityChange`
- `src/scene_git.rs` — `SceneGitStatus`, `SceneGitError`, status / commit / `pull_ff_only`, load HEAD scene
- `src/git.rs` — extended `WorldGitTracker` (tracked scene paths, refresh / commit / pull); OFPA paths unchanged
- `src/lib.rs` — modules and re-exports
- `tests/scene_diff.rs` — add entity, rename component
- `tests/scene_git.rs` — temp git repo: dirty, commit, diverged pull, worktree diff

### `redox_editor_ui`

- `src/git_overlay.rs` — scene status, object diff list, commit message, Commit / Pull (ff-only)
- `src/context.rs` — `scene_commit_message`, `track_scene_git`
- `src/dock.rs` — overlay actions → status channel
- `src/lib.rs` — export `GitOverlayAction`

### `redox_editor`

- `src/app/redraw.rs` — `git_refresh` (incl. scene), `git_scene_commit`, `git_scene_pull`
- `src/app/ui.rs` — Git menu commands; status bar branch + scene summary

### Docs

- `CHANGELOG.md` — Unreleased entry
- `docs/engineering/notes/EDT-C5-report.md` — this file

## Assumptions

1. **«stage scene+meta»** = stage the open `*.scene.ron` and `project.ron` (no per-scene `.meta` sidecar in A2).
2. **`SceneGitStatus.branch`** added for status bar (spec listed `path, dirty, ahead, behind`; branch is required for step 4 UX).
3. Scene tracking is bound via `EditorUiCtx::track_scene_git` when the host opens a project/scene; the main `app.rs` path does not open projects yet — integrator calls this when File/project flow lands.
4. Pull uses **local upstream** only (no network `fetch`); tests configure `branch --set-upstream-to`. Real remotes need a future fetch step before `pull_ff_only`.
5. Entities without `guid` in a file are skipped in object diff (loader assigns guids on save).

## Gaps / 📌

- **Distributed file locks** (EDITOR_PLAN optional stub): not implemented (📌).
- **Automatic `track_scene_git` on project open** in `redox_editor` main loop: API only until project UI slice wires it.
- **Network fetch** before pull: not in this slice.

## Coordinator — verify (no runs done here)

```bash
cd /home/besyak/Документы/redox/.worktrees/c5
cargo fmt --all
cargo clippy -p redox_editor_scene -p redox_editor_ui -p redox_editor --features editor-git -- -D warnings
cargo clippy -p redox_editor_scene -p redox_editor_ui -p redox_editor --no-default-features -- -D warnings  # scene git stubs
cargo test -p redox_editor_scene --features editor-git scene_diff scene_git
```

Expect: dirty/clean status on temp repo; one `Changed` on Name rename; commit clears dirty; diverged upstream → `SceneGitError::NotFastForward`; OFPA `WorldGitTracker` behavior unchanged.

## Step checklist

| Step | Done |
|------|------|
| 1 Scene file status | ✅ |
| 2 Object diff | ✅ |
| 3 Commit / pull ff-only | ✅ |
| 4 Overlay + docs | ✅ |
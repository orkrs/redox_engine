# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Not yet versioned.

## [Unreleased]

### Removed
- Cascaded shadow maps (CSM): `shadow/csm.rs`, cascade buffers/bindings and shader branches.
  Directional shadows are VSM-only. The `shadow_probe` example (a CSM probe) was deleted.

### Fixed
- ECS: `World::remove_component` now drops the removed value; `Column::replace` is panic-safe
  and `add_component` no longer double-drops when an old value's destructor panics.
- ECS: aliasing `&mut` in `Query::iter_mut`, duplicate component types in `spawn_bundle`,
  ZST alignment (found by the unsafe audit).

### Changed
- `ReadOnlyQueryData` is now an `unsafe trait`.
- Minimum supported Rust version is 1.88 (verified with `cargo check --workspace`).
- Line endings normalized to LF (`.gitattributes`).
- Large files split: editor `main.rs`, `viewport.rs`, `frame.rs`, VG `runtime.rs`, `vsm_clipmap.rs`, `prism_vault`.

### Added
- ECS: `Commands` / `CommandQueue` (deferred spawn, despawn, insert, remove, resources; ids reserved immediately); one-element bundles.
- Dependencies: thiserror 2, image 0.25, ron 0.12, notify 8, git2 0.21, rand 0.10, pollster 1, flume 0.12, hashbrown 0.17.
- CI: MSRV, Miri (ECS), docs (`-D warnings`), file-size check, weekly schedule; Dependabot.
- Tests: ECS ownership/drop tests, proptest model test, editor CLI tests.

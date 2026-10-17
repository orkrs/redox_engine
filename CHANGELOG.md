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
- ECS: `Query::par_for_each_mut` (parallel mutable iteration), `World::with_commands`.
- Core: system ordering (`SystemOrder` before/after labels, cycle detection) and `add_system_if` run conditions.
- Reflect: `#[derive(Reflect)]` (`redox_reflect_derive`) and the `FieldValue` trait.
- `missing_docs` enforced for math, input, ui, ecs, asset, core, reflect, physics, audio, vfx, anim, gameplay, scene; `clippy::unwrap_used` for library code.
- Integration tests for vfx, gameplay, scene, reflect, math, physics; ECS benchmarks (`docs/BENCHMARKS.md`).
- ECS: `Commands` / `CommandQueue` (deferred spawn, despawn, insert, remove, resources; ids reserved immediately); one-element bundles.
- Dependencies: wgpu 30, naga 30, winit 0.30, egui 0.36 (dock 0.21, plot 0.37), rapier3d 0.36, kira 0.12, glam 0.33; toolchain 1.96, MSRV 1.95; thiserror 2, image 0.25, ron 0.12, notify 8, git2 0.21, rand 0.10, pollster 1, flume 0.12, hashbrown 0.17.
- CI: MSRV, Miri (ECS), docs (`-D warnings`), file-size check, weekly schedule; Dependabot.
- Tests: ECS ownership/drop tests, proptest model test, editor CLI tests.

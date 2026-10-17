# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Not yet versioned.

## [Unreleased]

### Removed
- Old examples `showcase`, `prism_vault`, `ball_score`, `shadow_lab` (kept: `skinned_demo`, `dino_runner`, `dino_game`).
- Cascaded shadow maps (CSM): `shadow/csm.rs`, cascade buffers/bindings and shader branches.
  Directional shadows are VSM-only. The `shadow_probe` example (a CSM probe) was deleted.

### Fixed
- Render: meshes added through the asset flow used the raw vertex layout while the pipelines read the compressed one (invisible meshes); asset-flow materials sampled the legacy texture list instead of the handle pool.
- UI: every egui-based app (all examples except `skinned_demo`, and the editor) panicked at start since the egui 0.36 upgrade
  (`TexturesDelta` dropped unhandled); the deltas are now cleared after they are applied (`redox_ui`, editor).
- `dino_game` drew with a non-multisampled pass against MSAA pipelines (wgpu validation error); it now renders through `FrameView`.
- ECS: `World::remove_component` now drops the removed value; `Column::replace` is panic-safe
  and `add_component` no longer double-drops when an old value's destructor panics.
- ECS: aliasing `&mut` in `Query::iter_mut`, duplicate component types in `spawn_bundle`,
  ZST alignment (found by the unsafe audit).

### Changed
- winit: the closure event loop now runs through `ApplicationHandler` (`redox_core::window::RunClosure`);
  the eight file-wide `#![allow(deprecated)]` are gone (three narrow `create_window` opt-outs remain).
- `PakWriter` format is version 2 (per-entry raw size and codec); archives written by the first version are rejected.
- `ActionBinding` has new variants (`PadButton`, `PadAxis`); `ActionMap::evaluate` keeps its signature, `evaluate_with` takes all devices.
- `ReadOnlyQueryData` is now an `unsafe trait`.
- Minimum supported Rust version is 1.88 (verified with `cargo check --workspace`).
- Line endings normalized to LF (`.gitattributes`).
- Large files split: editor `main.rs`, `viewport.rs`, `frame.rs`, VG `runtime.rs`, `vsm_clipmap.rs`, `prism_vault`.

### Added
- Animation graph (`redox_anim::graph`): parameters and triggers, state machines with timed cross-fades and exit times, 1D blend trees with phase sync, masked layers, root motion, state-enter events; serde-friendly data.
- Game UI toolkit (`redox_ui::game`): widget tree with layout (column, row, stack, anchors, fill/fit), virtual 1920x1080 scale with safe area, localisation with fallback and plural forms (incl. Russian), focus navigation, screen stack, egui painter.
- Example `courier`: third-person prototype using all of the above plus a pak-loaded level and streamed KTX2 textures.
- Tests: golden frames for IBL from KTX2 and for a converged streamed texture; third-party KTX2/BasisLZ check (`REDOX_THIRD_PARTY_KTX2`).
- Textures: Basis Universal (UASTC, ETC1S/BasisLZ) in `.basis` and KTX2 with BC7/RGBA8 target choice; `redox_cook texture`, `texture-info`, `bake-ibl`, `pack --textures`; IBL from baked KTX2 cubes (`IBLResource::from_ktx2/export_ktx2`); cube/array bind layouts.
- Assets: priority task queue with cancel and a worker pool, range reads (`AssetSource::size/read_range`), `StreamedTexture`, uncompressed `.pak` entries.
- Render: `TextureStreamer` (floor mips first, mip choice from on-screen size, promotion/demotion with GPU copy, memory budget, hysteresis); the editor asset panel shows KTX2/Basis headers.
- KTX2: Zstandard supercompression, cube maps, 2D arrays and cube arrays, RGBA16F; `Ktx2Texture` now has `layers`/`faces` (`new_2d` for plain images), `to_ktx2_bytes`; `redox_asset::testing::Ktx2Builder`; `redox_cook texture-info`; `Texture::from_ktx2` keeps the texture shape (`Texture::view_dimension`).
- Render: the frame now runs from the compiled frame graph (`frame_pass_order`): `render_frame_into` is split into one method per pass and executed in graph order (golden frames unchanged).
- Render: `ShaderHotReload` (editor and `AppBuilder` with the `shader-hot-reload` feature) validates edited shaders and rebuilds 13 pass groups, now including tone mapping and the VG raster passes; verified by editing a real shader file under a running renderer.
- Assets/Render: KTX2 textures flow through `AssetManager` (built-in loader, previously the built-in loaders had no storage inserters and never delivered results), `sync_ktx2_textures_to_render` and `MaterialData::with_*_ktx2`.
- Render: `Renderer::reload_shaders` rebuilds the passes behind edited shaders without a restart (11 passes; tone mapping and VG passes excluded); `Texture::from_ktx2` uploads KTX2/BCn textures with their stored mips.
- Core: application states (`State`, `NextState`, `AppBuilder::add_state/on_enter/on_exit`, `in_state`); `SharedCommands` flushed by the dispatcher between stages.
- ECS: component hooks `World::on_add` / `World::on_remove` (spawn, add, remove, despawn).
- Asset: `retain`/`release` ownership with unloading, `add_dependency` and `status_with_dependencies`, loading through a `.pak`/folder source, per-frame completion budget, KTX2/BCn loader, LZ4 compression in `.pak`; `redox_cook pack --compress --incremental`.
- Scene: binary `world.bin` (`world_bin`: postcard, cell index, checksums, OFPA conversion).
- Input: gamepads (`GamepadsState`, gilrs backend, `PadButton`/`PadAxis` bindings), text entry with IME (`TextInput`), layered `InputContexts`; the app polls pads and toggles the platform IME.
- Render: `pipeline_cache` (on-disk Vulkan pipeline cache, saved when the renderer drops); `frame_graph` declares the frame as a `RenderGraph` and a test keeps it in sync with `renderer/frame.rs`.
- Benchmarks: animation (`redox_anim`) and virtual-geometry build (`redox_render`); `missing_docs` also in `redox_render`, `redox_editor_core`, `redox_editor_ui`.
- Docs: `PROJECT_MAP.md`, `ARCHITECTURE_REALITY.md`, `PROGRESS.md` rewritten against the current code.
- Scene: `SaveManager` (crash-safe, versioned save slots with migrations and backups).
- Input: runtime rebinding (`ActionMap::rebind`, conflicts, RON persistence, `Rebinder` capture).
- Asset: `AssetGuid` + `.meta` sidecars + `AssetRegistry`; `.pak` archives (`PakWriter`/`PakReader`) and `AssetSource` (`DirSource`, `PakSource`); new `redox_cook` CLI (scan/pack/list/verify).
- Render: `render_graph` compiler (ordering, culling, lifetimes, texture aliasing, DOT dump); not yet used by the renderer.
- ECS: `Query::par_for_each_mut` (parallel mutable iteration), `World::with_commands`.
- Core: system ordering (`SystemOrder` before/after labels, cycle detection) and `add_system_if` run conditions.
- Reflect: `#[derive(Reflect)]` (`redox_reflect_derive`) and the `FieldValue` trait.
- `missing_docs` enforced for math, input, ui, ecs, asset, core, reflect, physics, audio, vfx, anim, gameplay, scene; `clippy::unwrap_used` for library code.
- Integration tests for vfx, gameplay, scene, reflect, math, physics; ECS benchmarks (`docs/BENCHMARKS.md`).
- ECS: `Commands` / `CommandQueue` (deferred spawn, despawn, insert, remove, resources; ids reserved immediately); one-element bundles.
- Dependencies: wgpu 30, naga 30, winit 0.30, egui 0.36 (dock 0.21, plot 0.37), rapier3d 0.36, kira 0.12, glam 0.33; toolchain 1.96, MSRV 1.95; thiserror 2, image 0.25, ron 0.12, notify 8, git2 0.21, rand 0.10, pollster 1, flume 0.12, hashbrown 0.17.
- CI: MSRV, Miri (ECS), docs (`-D warnings`), file-size check, weekly schedule; Dependabot.
- Tests: ECS ownership/drop tests, proptest model test, editor CLI tests.

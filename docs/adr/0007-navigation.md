# ADR 0007: Navigation (navmesh bake, pathfinding, local avoidance)

Status: proposed (2026-10-17, research by Claude, needs owner confirmation of the open questions)

Plan item H0 (`docs/PLAN_EXECUTION.md` stage H, `docs/PLAN_PHYSICS_NAV.md` part 2). Research only, no code.
All crate facts below were read from the crates.io API, docs.rs and GitHub pages on 2026-10-17.
"unverified" marks what could not be confirmed from those pages. Source code of the crates was not
downloaded or run.

## Context

The engine needs three separate things (see the plan, part 2):

1. **Bake**: Recast-style tiled navmesh from level geometry (voxelise, regions, contours, polygons,
   detail height mesh), offline in `redox_cook` and at runtime per tile.
2. **Pathfinding**: A* over polygons plus funnel, area costs, off-mesh links, several agent classes,
   async queries.
3. **Avoidance**: ORCA/RVO for crowds, target 200 agents in under 1 ms.

Host constraints: Windows + Vulkan, Rust 1.96 / MSRV 1.95, edition 2024, `glam 0.33`, `rapier3d 0.36`,
own ECS (ADR 0002), no Bevy. Licences allowed by `deny.toml`: MIT, Apache-2.0 (+LLVM exception),
BSD-2/3, ISC, Zlib, MPL-2.0, CC0, BSL-1.0 and a few font/Unicode ones. `multiple-versions = "warn"`
(two glam versions in the tree warn, they do not fail).

## Candidates

Numbers: crates.io total downloads (recent 90 days in brackets where useful). "glam" is the version
the crate requires; ours is 0.33, so every candidate needs a copy-conversion at the boundary.

| Candidate | Latest / last release | Licence | Stars, last push | Role | glam | Bevy / C++ | Notes |
|---|---|---|---|---|---|---|---|
| `rerecast` | 0.4.0, 2026-08-05 | MIT OR Apache-2.0 | 170, 2026-08-05 | bake (Recast port) | ^0.32 | core crate has no Bevy dep (bevy_reflect is optional); pure Rust (no FFI seen) | Voxel pipeline, watershed/monotone/layer partitioning, detail mesh, `AreaType` + `ConvexVolume` area marking, `tile_size`/`border_size` in `Config` ("multi-tile meshes"), `no_std` + `libm` for cross-platform determinism. 52k downloads. No off-mesh connections, no runtime query layer, no tile cache documented. Tile building API not documented on docs.rs (unverified how ergonomic). `unsafe` use unverified. Bevy integration (`bevy_rerecast`) is separate and unneeded. |
| `landmass` | 0.9.3, 2026-10-11 | MIT OR Apache-2.0 (GitHub shows Apache-2.0) | 115, 2026-10-11 | pathfinding + avoidance + path simplification | ^0.30.5 | engine-agnostic core (`bevy_landmass` is a separate crate); pure Rust | Archipelago holds Islands (one navmesh each), Agents, Characters, `AnimationLink` (off-mesh links with permitted-link filter), polygon type indices with per-type costs, configurable coordinate system (`XYZ`/`XY`), height mesh for stairs, built-in avoidance via `dodgy_2d`. Takes polygons + vertices, so rerecast output feeds it. 32k downloads, active (3 releases in 12 months). Does not move agents, game feeds position/velocity each frame. Deps: `geo`, `kdtree`, `disjoint`, `slotmap`. Tiles: not a first-class concept, islands link by edges within a hardcoded 0.01 distance (source TODO says "make configurable"); tile rebuild = replace an island (inferred, unverified). Multiple agent radii in one archipelago unverified. Perf for 200 agents unverified. `unsafe` unverified. |
| `polyanya` | 0.17.1, 2026-08-25 | MIT OR Apache-2.0 | 523, 2026-10-13 | pathfinding only (any-angle, exact shortest) | ^0.32 | no Bevy; depends on `rerecast` 0.4 (also dev), `spade`, `geo`, `rstar` | Layers (overlapping navmeshes: floors, bridges), per-layer cost, enable/disable layers. No area costs within a layer, no avoidance, no off-mesh links documented, no agent radius (mesh must already be eroded). Not a funnel-based search, it replaces A*+funnel. 99k downloads. |
| `oxidized_navigation` | 0.12.0, 2024-12-25 | MIT OR Apache-2.0 | 197, archived 2025-11-14 | bake + find_path | n/a (Bevy) | Bevy plugin, tied to parry3d version | Tiled async regeneration on collider change, rapier/avian adapters. **Archived and deprecated**, author points to rerecast. Rejected. |
| `recastnavigation-sys` (+ `recast_navigation` 0.1.0 wrapper, 2022) | sys 1.0.3, 2024-01-11 | MIT (bundled C++ Recast/Detour is zlib upstream, not re-verified here) | 8, unknown | bake + Detour + crowd via FFI | none (C API) | C++ built from vendored submodule (needs a C++ toolchain, cmake/bindgen use unverified) | Most complete feature set by definition (tile cache, off-mesh, crowd) but raw unsafe FFI, a C++ build on every dev machine and CI, and a thin 1.0.3 sys crate with a stale safe wrapper. Rejected as primary, kept only as a reference oracle for tests (not a dependency). |
| `recast-rs` workspace (`landmark`, `waymark`, `waymark-crowd`, `waymark-tilecache`, `waymark-dynamic`), github.com/wowemulation-dev | 0.1.x, `waymark-crowd` 0.1.0 2026-02-23 (56 downloads) | Apache-2.0 / MIT | 9, 2026-04-20 | full Recast+Detour+crowd+tile cache port | glam (version unverified) | pure Rust, wasm-capable, MSRV 1.92 badge | Claims every feature we want (areas, off-mesh, tile cache, obstacles, crowd), validated against C++ counts on 3 meshes. One part-time author, 9 stars, tiny usage, weeks old. Too risky as a dependency, worth re-checking in 6 months. `unsafe` unverified. |
| `dodgy_2d` | 0.5.5, 2025-04-24 | MIT OR Apache-2.0 | dodgy repo 38 stars, last commit 2025-04-24 | ORCA avoidance, 2D | ^0.29.1 | none; deps only `glam`, `rand` | 29k downloads. Used by landmass. Agents on a ground plane, which is what navmesh agents are. Quiet for 18 months (stable, dependabot only). |
| `dodgy_3d` | 0.5.2, 2024-12-15 | MIT OR Apache-2.0 | same repo | ORCA, 3D | ^0.29.1 | none | 5k downloads. Only needed for flying or swimming agents. |
| `navmesh` (PsichiX) | 0.12.1, 2021-11-30 | not checked | last commit 2021-11-30 | grid/mesh pathfinding | n/a | none | Unmaintained for 5 years. Rejected. |
| `pathfinding` | 4.16.0, 2026-09-07 | MIT OR Apache-2.0 | 3.1M downloads | generic A*/Dijkstra over any graph | none | none | Not navmesh aware (no funnel, no points). Usable only as the A* kernel of a hand-written solution; the A* itself is about 80 lines, so it is not worth a dependency. |
| `rvo2` crate | no crate found by the search `rvo2` | - | - | - | - | - | Nothing to evaluate. |
| Write everything ourselves | - | ours | - | all three | 0.33 | none | See below. |

Not evaluated (no evidence they fit): `vleue_navigator` (Bevy plugin over polyanya), `bevy_flowfield_tiles_plugin`,
`seldom_map_nav` (Bevy, 2D tilemaps), `raasta` (1.0.0, 288 downloads, AGNOS-specific, unverified).

## Decision

Take two existing crates and write the glue plus the missing pieces ourselves. New crate `redox_nav`
owns the public API, so the dependencies stay behind it and can be swapped.

| Part | Decision | Why |
|---|---|---|
| Bake | **Take `rerecast` 0.4** as the Recast kernel. Write in `redox_nav` the tile driver (cut level geometry per tile with a border, build each tile, collect polygons), area marking from our scene data, the `.nav` cooked format and the determinism test. | The only maintained, pure-Rust, engine-independent Recast port with areas and a detail mesh and a permissive licence. Writing voxelisation, watershed and contours ourselves is the 8-12 day item with the most subtle bugs; reuse saves most of it. |
| Pathfinding | **Wrap `landmass` 0.9 behind `redox_nav::NavWorld`** for the first implementation, time-boxed spike (see H2). Off-mesh links map to its `AnimationLink`, area costs to polygon types. Own the query API (`find_path`, `nearest_point`, `raycast`, `random_point_in_radius`) so the backend can be replaced. | Gives A*, path simplification, off-mesh links, typed costs and avoidance in one engine-agnostic MIT/Apache crate, active in the last week. |
| Avoidance | **Use `dodgy_2d` through landmass first**; if landmass is rejected, use `dodgy_2d` directly (or write ORCA, about 300-400 lines, in `redox_nav`). `dodgy_3d` only if flying agents appear. | ORCA is a solved algorithm and `dodgy_2d` is small, dependency-light and licence-clean. |

glam mismatch: rerecast wants 0.32, landmass and dodgy 0.30 / 0.29, we have 0.33. This is not a blocker:
cargo keeps several glam versions, `deny` only warns. `redox_nav` converts `[f32; 3]` at the boundary
(vertex buffers are plain arrays, the cost is a copy per bake or per island, not per frame; per frame
agent position/velocity are 3 floats each). We do not re-export foreign glam types. Bumping is a PR
to the upstream crates, not our job.

Why not "write everything ourselves": 3-5 weeks of plan time for work that two permissive crates
cover; the risk is not algorithms but long-tail bake bugs (thin walls, stairs, region merging), which
rerecast has already hit. Why not C++ Recast/Detour via FFI: unsafe surface, C++ toolchain on CI, stale
wrappers. Why not `recast-rs` workspace: most complete on paper but a weeks-old one-person project.

## Fallback plan

Each part has its own exit, decided by a concrete test in the matching plan item:

- **rerecast fails** (tiles seams, non-deterministic output, bad detail mesh, abandoned): fork it
  (MIT/Apache allows this, pure Rust, 520 commits) and bump glam ourselves; second choice is the
  `recast-rs` workspace (`landmark`) if it matures; last choice is writing the bake (adds ~2 weeks
  to H1). The `redox_nav` bake output type (`NavTile { verts, polys, area ids, detail }`) is our own,
  so consumers do not change.
- **landmass fails** (200 agents over 1 ms, no usable tile replace, one agent radius per archipelago and
  we need classes, hits unsafe/`geo` weight problems): replace with our own polygon graph
  (A* with a binary heap, funnel with portal edges, about 600 lines, `polyanya` as an optional
  any-angle search since it consumes rerecast meshes) and call `dodgy_2d` directly. The `NavWorld`
  API stays. Cost: about +4 days on H2/H3.
- **dodgy_2d fails** (stalls in dense crowds): write ORCA ourselves or fall back to simple
  separation + flow steering for crowds and ORCA only for near neighbours.

## First implementation steps (adjusted plan items)

**H1, tiled bake (plan 8 days, now about 6 with rerecast).**
1. New crate `redox_nav` (workspace member; add `rerecast` with `default-features = false`, features
   `libm` and `serde` only; no `bevy_*`). Run `cargo deny check` and `cargo tree -d` before merging,
   record the added crates in `docs/CRATES.md`.
2. Day 1 spike: bake one hard-coded test room and a staircase from raw triangles into polygons +
   detail mesh; confirm tile building with `tile_size`/`border_size`/`aabb` works and that two
   neighbouring tiles share edges. Gate: if seams or API gaps appear, apply the rerecast fallback.
3. Geometry source: triangles from `redox_scene` static meshes and `redox_physics` colliders (convert
   rapier shapes to triangle soup via parry `to_trimesh`), world-cell based tiling aligned with
   scene cells so streaming a cell loads/bakes its tiles. Area ids from a per-mesh/volume component
   (`NavArea`), "not walkable" from `AreaType::NOT_WALKABLE`.
4. Cooked `.nav` format (versioned, per-tile blobs, in the `.pak`), `redox_cook bake-navmesh`.
5. Runtime: rebuild a tile on change on the worker pool, swap atomically; dynamic obstacles in H1 are
   handled as "geometry marked not walkable and tile rebuilt"; real carving is deferred to H3.
6. Tests (as in the plan): room with a doorway, stairs within step height, obstacle splits coverage,
   bit-identical bake across two runs (and across threads), tile seam test.

**H2, pathfinding (plan 6 days).**
1. `NavWorld` facade: `find_path`, `nearest_point`, `raycast_navmesh`, `random_point_in_radius`,
   returning our types (`glam 0.33`).
2. Spike with landmass first (2 days, time-boxed): feed baked tiles as islands, check (a) tile
   replace without agents teleporting, (b) per-polygon-type costs, (c) off-mesh link traversal,
   (d) one archipelago per agent class (small/large radius, each baked with its own `walkable_radius`),
   (e) 1000 path queries and timings. Go/no-go against the fallback above.
3. Off-mesh links as authored components (`NavLink { from, to, bidirectional, class_mask }`) mapped
   to the backend; async queries on workers with a per-frame budget; cache by (start poly, end poly).
4. Tests: known-map optimality, unreachable returns `None`, off-mesh works, area cost changes route.

**H3, avoidance (plan 6 days, now about 4-6).**
1. Agent systems in the own ECS (`NavAgent`, `NavTarget`, `NavPath`; events `PathReached`/`PathFailed`),
   fixed-step update, desired velocity from the backend, then move through the physics
   character controller (rapier kinematic), and write the real velocity back each tick.
2. Benchmark 200 agents (criterion, release) against the `< 1 ms` goal on the reference machine;
   record the number in this ADR when measured. Check single-threaded first; ORCA per agent is
   independent, so `rayon`-style parallelism is the escape hatch.
3. Two-agents-head-on, crowd-through-door and 200-agent no-stuck tests.
4. Carving of dynamic obstacles stays on the tile-rebuild path from H1 (temporary not-walkable
   volumes), no separate tile cache.

## Risks

- Maintenance: rerecast and landmass are each effectively a one-maintainer project (janhohenheim,
  andriyDev); the two are loosely coupled (landmass_rerecast exists for Bevy). Mitigation: vendor-able
  licences, our own facade and cooked format.
- glam version split (0.29 / 0.30 / 0.32 / 0.33) adds duplicate crates and boundary copies; the
  `multiple-versions` warnings will grow.
- Tile seams and island linking are the least-documented area of both crates (rerecast tile API not
  on docs.rs; landmass island edge linking only seen in source via a hardcoded 0.01 distance).
- Performance of landmass avoidance at 200 agents is unmeasured; the plan's 1 ms goal is unproven.
- `unsafe` and `forbid(unsafe_code)` status of rerecast, landmass, dodgy: unverified.
- Voxel bake of big terrain (PLAN_WORLD) at runtime may exceed the frame budget; needs the worker pool
  and coarse cell size per tile.
- Determinism across machines depends on rerecast's `libm` feature actually covering all float math
  (claimed in README, unverified).
- `recast-rs` could mature into a single full replacement; re-evaluate before H3.

## Open questions for the owner

1. Is a second glam in the dependency tree (and a `cargo deny` multiple-versions warning) acceptable,
   or should we require upstream bumps before adopting?
2. Do we need agents that fly or swim (then `dodgy_3d` and a volumetric approach are in scope)?
3. How many agent classes at launch (one radius, or small/medium/large)? Each class costs a bake.
4. Runtime bake budget: is it acceptable to ship pre-baked tiles only for the first game, with
   runtime rebuild only for small dynamic changes?
5. Should off-mesh links be authored in the editor in H2 or can they be code/scene-file only?
6. Is the 1 ms / 200 agents target for one core or for the whole worker pool?

## Sources

crates.io API pages for rerecast, landmass, polyanya, dodgy_2d, dodgy_3d, pathfinding, oxidized_navigation,
recastnavigation-sys, waymark-crowd; GitHub repos janhohenheim/rerecast, andriyDev/landmass,
vleue/polyanya, andriyDev/dodgy, TheGrimsey/oxidized_navigation, andriyDev/recastnavigation-rs-sys,
wowemulation-dev/recast-rs; docs.rs pages for rerecast 0.4.0 and landmass 0.9.3.

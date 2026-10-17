# RedOx Engine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)

RedOx Engine is a **modular**, **data‑oriented** game engine written in Rust. It combines modern GPU‑driven rendering techniques with a lightweight, explicit CPU architecture built around a custom archetype ECS. The engine is designed as a Cargo workspace with focused, loosely coupled crates that have clear one‑way dependencies – this keeps compilation times reasonable, ownership boundaries explicit, and systems testable in isolation.

This README documents the **current state** of the engine and serves as a starting point for contributors and curious developers.

---

## Table of Contents

1. [What this engine is](#1-what-this-engine-is)
2. [What this engine is not](#2-what-this-engine-is-not)
3. [Workspace structure](#3-workspace-structure)
4. [Core concepts and terminology](#4-core-concepts-and-terminology)
5. [Rendering pipeline overview](#5-rendering-pipeline-overview)
6. [Temporal AA and supersampling defaults](#6-temporal-aa-and-supersampling-defaults)
7. [Shadows overview](#7-shadows-overview)
8. [Clustered forward lighting](#8-clustered-forward-lighting)
9. [Virtual Geometry (Nanite‑like)](#9-virtual-geometry-nanite-like)
10. [Materials and PBR](#10-materials-and-pbr)
11. [Post processing](#11-post-processing)
12. [Physics integration](#12-physics-integration)
13. [Audio integration](#13-audio-integration)
14. [UI and debugging](#14-ui-and-debugging)
15. [Assets and resource flow](#15-assets-and-resource-flow)
16. [Examples](#16-examples)
17. [How to build](#17-how-to-build)
18. [How to run](#18-how-to-run)
19. [Performance goals](#19-performance-goals)
20. [Conventions](#20-conventions)
21. [Troubleshooting](#21-troubleshooting)
22. [Roadmap](#22-roadmap)
23. [Tech stack](#23-tech-stack)
24. [License](#24-license)

---

## 1) What this engine is

RedOx is a practical engine for experiments and real‑time demos.  
It prioritizes **modern rendering techniques** that work well on today’s GPUs.  
It is designed around **explicit data flow** and favours **stable, predictable code paths**.  
It follows a “measure first, then optimize” workflow.  

- **GPU backend**: [`wgpu`](https://wgpu.rs/) (Vulkan, DX12, Metal)  
- **CPU side**: lightweight, built on a custom ECS  
- **Physics**: [`rapier3d`](https://rapier.rs/) integration  
- **Examples** act as living integration tests and are meant to be **readable and hackable**  

The engine aims to keep frame times stable and code approachable.

---

## 2) What this engine is not

RedOx is **not** a full editor‑first production engine (yet).  
It does **not** ship a complete asset pipeline with hot‑reload for every asset type.  
It does **not** try to be a Unity or Unreal replacement.  
It does **not** hide GPU concepts behind heavy abstraction.  
It does **not** currently include:  

- Networking gameplay systems  
- A full animation system for skinned characters  
- A scripting language layer  
- An audio authoring toolchain  

RedOx is **deliberately small, explicit, and engineer‑friendly** – it is meant to be read, modified, and evolved.

---

## 3) Workspace structure

The repository is a Cargo workspace. Each domain lives in its own crate with a minimal API surface.

### `crates/redox_core`
- Contains runnable examples and the glue that composes systems.  
- Creates windows via `winit`, initializes the renderer, physics, and drives the main loop.  
- The starting point for exploring the engine.

### `crates/redox_render`
- The GPU renderer: owns `wgpu` device creation, surface management, and all render passes.  
- Implements clustered forward lighting, TAAU, GTAO, virtual shadow maps (VSM) for the sun, local shadow atlas, and Virtual Geometry.  
- Contains the shader source manager and GPU‑side structs for lights and materials.

### `crates/redox_ecs`
- The custom archetype ECS with cache‑friendly storage, tuple queries, parallel iteration, events, and hierarchies.  
- Used by core examples and subsystems.

### `crates/redox_math`
- Wraps math types and utilities (built on `glam`): vectors, matrices, transforms, camera helpers, frustum culling.

### `crates/redox_physics`
- Integrates `rapier3d`: defines rigid‑body components, synchronisation between ECS `Transform` and Rapier state, and a `PhysicsContext` resource.

### `crates/redox_audio`
- Handles audio playback (via `kira`), exposes debug hooks for visualisation, and stays independent of rendering.

### `crates/redox_ui`
- Provides debug UI integration using `egui` and `egui_wgpu`. Optional at runtime, mostly used in examples.

---

## 4) Core concepts and terminology

| Term                | Description |
|---------------------|-------------|
| **Frame**           | One iteration of the main loop. |
| **Pass**            | A stage of the render pipeline (e.g., shadow pass, forward pass). |
| **Resource**        | A GPU object like a buffer, texture, or sampler. |
| **Handle**          | An index‑like reference to an uploaded GPU resource. |
| **Material**        | PBR parameter set plus texture bindings. |
| **Mesh**            | Vertex and index data. |
| **Virtual Mesh**    | A Virtual Geometry asset rendered through meshlets. |
| **Cluster**         | Screen‑space cell used for light culling. |
| **History**         | Previous TAA output used for temporal accumulation. |
| **Velocity buffer** | Stores motion vectors for temporal reprojection. |
| **Local shadow atlas** | Tiled depth texture for point‑light shadows (4×2 grid). |
| **VSM page pool**   | Layered R32Float pages backing the sun's virtual shadow map clipmap. |

---

## 5) Rendering pipeline overview

RedOx uses a **forward pipeline** with clustered lighting.  
A depth pre‑pass is used where needed.  
The engine outputs HDR lighting to an FP16 render target.

**High‑level pass order:**

1. Update camera and light uniforms.  
2. Update clustered light buffers.  
3. Render shadow maps (VSM pages for the sun + local atlas).  
4. Render main HDR color buffer (forward pass).  
5. Render normal buffer.  
6. Generate GTAO (bent normal + ambient occlusion).  
7. Generate velocity buffer.  
8. TAA resolve.  
9. Tone mapping (with constrained sharpening).  
10. Present to the surface.

The pipeline is designed for stability, predictability, and easy extension.

---

## 6) Temporal AA and supersampling defaults

The engine implements **TAAU‑style control** with two resolutions:

- `internal_width/height` – the resolution at which the scene is rendered.  
- `config.width/height` – the output resolution (swapchain size).

By default **`internal_scale = 1.70`** – the scene is rendered at 170% of the output resolution in each axis.  
This acts as **supersampling (SSAA)** combined with temporal accumulation, producing an exceptionally clean image without relying on blur.

- **Blend factor**: `0.05` – the current frame contributes only 5% in steady state; 95% comes from history.  
- **Jitter**: Halton sequence (base 2/3) for subpixel distribution.  
- **Velocity buffer**: generated from depth and matrices (per‑vertex motion not yet implemented).  
- **Ghosting control**: neighborhood clamping in YCoCg (3×3 window), variance clipping on luma, Catmull‑Rom reconstruction.

---

## 7) Shadows overview

The engine supports both directional and point light shadows.

### Directional shadows – Virtual Shadow Maps (VSM)

- A clipmap of light-space levels around the camera; each level is a grid of 256 px pages.  
- Pages are allocated on demand from a shared pool and cached until invalidated.  
- A screen-space pass traces the clipmap and PBR samples the filtered result.  
- The former cascaded shadow maps were removed (see `CHANGELOG.md`).

### Point light shadows – local shadow atlas

- Six faces per light are rendered into a **4×2 tiled atlas** (tiles 1024×2048 each).  
- Each face uses its own view‑projection matrix.  
- The shader selects a face based on light direction, maps UVs to the atlas tile, and samples with PCF.

---

## 8) Clustered forward lighting

- Screen is subdivided into 16×16 pixel clusters.  
- Depth is subdivided into 24 slices (logarithmic distribution).  
- For each cluster a list of affecting point lights is built on the CPU.  
- In the PBR shader, each fragment looks up its cluster and iterates only over the relevant lights.

**Cluster buffers**:

- `point_lights` – storage buffer with all lights.  
- `cluster_metadata` – per‑cluster offset and count.  
- `cluster_light_indices` – flat array of light indices.  
- `cluster_info` – uniform with grid dimensions and depth parameters.

---

## 9) Virtual Geometry (Nanite‑like)

The renderer includes an experimental **Virtual Geometry** pipeline:

- Meshes are split into meshlets (clusters of triangles).  
- CPU‑side frustum culling per meshlet.  
- LOD selection based on projected size.  
- Indirect drawing via `draw_indexed_indirect`.  
- Entities with a `VirtualMesh` component are rendered through this pipeline.

---

## 10) Materials and PBR

- Physically based shading with **Cook‑Torrance BRDF**.  
- Supports albedo, metallic, roughness, normal maps, and image‑based lighting (IBL).  
- Materials are defined as `MaterialData` on the CPU and uploaded to GPU uniform buffers.  
- Textures are referenced via handles and resolved at draw time.

---

## 11) Post processing

- **HDR rendering** to FP16 texture.  
- **Constrained sharpening** (contrast‑adaptive, clamped) applied just before tone mapping.  
- **Tone mapping** using a simple Reinhard‑like curve.  
- **GTAO**: samples the depth/normal prepass with a noise texture at reduced resolution, then blurs and upsamples the result; the pass is skipped (AO = 1.0) when disabled.

---

## 12) Physics integration

- Physics uses `rapier3d`.  
- Physics is stepped explicitly each frame.  
- `PhysicsContext` resource holds the Rapier world.  
- ECS components (`RigidBody`, `Collider`) link entities to physics bodies.  
- Transforms are synchronised **to** physics before stepping and **from** physics after stepping.

---

## 13) Audio integration

- Audio is handled by `redox_audio` using the `kira` library.  
- The audio context is independent of rendering.  
- Audio systems can publish debug draw data (e.g., lines for occlusion rays).  
- Designed for future extension with spatialisation and occlusion.

---

## 14) UI and debugging

- **Debug UI** is built with `egui` and wrapped by `redox_ui`.  
- UI is rendered as an overlay after the main pass.  
- Provides panels for controls, metrics, and visualisation (e.g., cluster stats, shadow atlas).  
- Also supports drawing **debug lines** directly in 3D space.

---

## 15) Assets and resource flow

- Simple MVP asset system: `AssetManager` and `Handle<T>`.  
- Meshes can be procedural or loaded (e.g., OBJ via `tobj`).  
- Textures are loaded via the `image` crate.  
- Materials can reference texture handles.  
- GPU resources are stored in vectors; handle IDs are mapped to indices.

---

## 16) Examples

The engine provides several runnable examples that demonstrate its features and act as regression tests.  
All examples are located in `crates/redox_core/examples` and `crates/redox_render/examples`.  
You can run any example with:

```bash
cargo run --example <example_name> -p redox_core
```

### Featured example: `dino_runner`

Voxel endless-runner scene in a sunset canyon style:

- Manual jump controls: `Space` / `Up` / `W`
- Endless score with speed ramp
- Collision -> `Game Over`
- Restart run: `R`

Run it with:

```bash
cargo run --example dino_runner -p redox_core
```

## 17) Build and run

Requires a stable Rust toolchain and a GPU with Vulkan, DX12 or Metal.

```bash
cargo check --workspace
cargo test -p redox_ecs -p redox_audio -p redox_render -p redox_anim
cargo run --example dino_runner -p redox_core
cargo run --example skinned_demo -p redox_core --release  # skeletal animation + IK
```

`skinned_demo --screenshot <dir>` renders the scene offscreen (no window) and writes PNG frames.

Generate a procedural world with the PCG CLI:

```bash
cargo run -p redox_scene --bin pcg -- --out worlds/arena --kind arena --width 6 --height 6 --cell-size 16 --seed 42
```

With the `shader-hot-reload` feature (`redox_render`, or `redox_editor` which forwards it) WGSL is
read from disk at startup, so editing a shader needs only an app restart, not `cargo build`. The editor
also polls `shaders/` and logs compile errors when a file is saved; pipelines are not swapped live.

## 18) Status and known limitations

See `ARCHITECTURE_REALITY.md` and `PROGRESS.md` for the current state of each system.
Known gaps: DDGI is a GPU probe fill against scene AABBs with an analytic sky/sun model (no
light bounce from surfaces), shader hot-reload does not swap pipelines live, directional VSM
shadows cache pages and do not follow in-place mesh deformation (point-light shadows do), skinning
runs on the CPU, and normal maps are not skinned. `redox_editor --bake` runs the meshlet/SDF
bakes headlessly.

## 19) Rendering diagnostics and quality tools

- **GPU timings.** `Renderer::gpu_profiler` writes timestamps between pass groups (needs the
  `TIMESTAMP_QUERY` features, requested automatically when the adapter has them);
  `gpu_profiler.report()` prints `shadow_passes 0.07 | gtao 0.22 | forward 0.33 | total ...`.
- **Headless frames.** `skinned_demo --screenshot <dir> [--size WxH] [--times 5,10]` renders
  offscreen and writes PNGs. The offscreen path now matches the window (sun shadows used to stay
  off until the first resize because of a shadowed `vsm_info_buffer`).
- **Golden frames.** `skinned_demo --golden crates/redox_core/tests/golden` renders three fixed
  frames and compares them with the stored images (exit code 1 on a mismatch);
  `--update-golden` rewrites the references after an intentional change.
- **Sun shadows** come from a virtual shadow map clipmap (5 levels from 16 m, 256 px pages).
  A compute pass (`vsm_request.wgsl`) tells the CPU which pages the visible pixels need, the
  pool evicts the least recently used page, and windows follow the camera in page steps.
- **Look.** Tone mapping writes linear values to sRGB targets (it used to be encoded twice);
  `Renderer::set_ibl_intensity` scales environment lighting.

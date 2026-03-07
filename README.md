# RedOx Engine

**RedOx Engine** – A high‑performance modular game engine written in Rust.
The name *RedOx* symbolises transformation and energy, reflecting the engine’s focus on speed, flexibility, and modern rendering techniques.

## Core Architecture

- **Modular workspace** – The engine is split into independent crates (`redox_math`, `redox_ecs`, `redox_render`) with strict one‑way dependencies, promoting clean system boundaries.
- **Custom ECS (archetype‑based)** – A cache‑friendly entity‑component system designed for zero‑allocation in the hot loop. It supports hierarchical entities, multithreaded queries, and a double‑buffered event system.
- **GPU‑driven rendering (wgpu)** – A modern, robust graphics foundation designed for cross-platform compatibility. The renderer currently supports a dynamic forward shading pass with multi-light capabilities and texture mapping.
- **Event‑driven communication** – Modules interact through a global event system (double‑buffered, thread‑safe), ensuring loose coupling and safe parallel execution.

## Current Progress

The foundational pillars of the RedOx Engine successfully reached their Phase 1 completion state:

- **Mathematical foundation** (`redox_math`) – **Complete & Tested**
  Offers all necessary geometric primitives, high-performance transformation utilities (via `glam`), and complex frustum culling logic using the Gribb-Hartmann method. Completely covered by integration tests.
- **Core ECS** (`redox_ecs`) – **Complete & Tested**
  A fully featured, archetype‑based ECS featuring entity generation lock-free queues, component storage pooling, parallel disjoint iteration, global events, and parent-child hierarchy support.
- **Renderer Module** (`redox_render`) – **Complete & Optimized**
  A robust `wgpu`-based renderer achieving seamless ECS integration. Features include a dynamic forward shading pass with **GPU-driven instancing** (via storage buffers), procedural meshes, textured materials, and a real-time multi-light system (Directional + Point lights with attenuation).
- **Physics Engine** (`redox_physics`) – **Integrated**
  Powered by `rapier3d`, providing rigid body dynamics, colliders, and seamless synchronization with the ECS `Transform` components. Full support for gravity, restitution, and kinematic bodies.

## Key Features

- **Optimised 3D Math** – Built on [`glam`](https://crates.io/crates/glam) for zero‑cost abstractions and SIMD acceleration.
- **Geometric Primitives & Culling** – Axis‑aligned bounding boxes (`Aabb`), bounding spheres (`Sphere`), planes (`Plane`), and real-time Frustum Culling against model matrices.
- **High‑Performance ECS**:
    - Archetype‑based storage for linear cache‑friendly iteration.
    - Parallel queries via [`rayon`](https://crates.io/crates/rayon).
    - Double‑buffered events with independent thread-safe readers.
    - Hierarchical entities (parent‑child) for scene graph building.
- **RedOx Render (wgpu)**:
    - Asynchronous `wgpu` (v0.20) context initialization tightly integrated with `winit`.
    - Single-pass forward rendering pipeline supporting depth testing and backface culling.
    - Procedural mesh generation (Cube, Sphere, Torus, Quad) and external `.obj` loading.
    - Seamless 2D texture mapping leveraging the `image` crate.
    - Dynamic Lighting: Support for a primary Directional Light and multiple distance-attenuated Point Lights rendered simultaneously.
    - Built-in ECS components (`Transform`, `MeshHandle`, `MaterialHandle`) connecting gameplay to the GPU layer.
- **Continuous Verification** – Deep integration and unit tests verify the correctness of mathematical operations, ECS mechanics, and procedural geometry logic.

## Examples

To see the engine in action, run the following command from the workspace root:

```bash
cargo run --example falling_balls -p redox_core
```
*(This "Unified Showcase" example demonstrates the full power of the engine: 100+ physics-enabled spheres falling into a box, synchronized via ECS, rendered using GPU instancing, and illuminated by dynamic point lights.)*

```bash
cargo run --example basic_cube -p redox_render
```
*(A basic example demonstrating window creation, GPU context initialization, procedural mesh generation, texture loading, material application, and real-time lighting.)*

## Tech Stack

| Category       | Libraries / Tools                                                                 |
|----------------|------------------------------------------------------------------------------------|
| Language       | Rust 2024 edition                                                                  |
| Linear Algebra | [`glam`](https://crates.io/crates/glam)                                           |
| ECS            | **Custom** (`redox_ecs`: archetype‑based, zero‑alloc in hot path)                 |
| Parallelism    | [`rayon`](https://crates.io/crates/rayon) (ECS queries)                           |
| Rendering      | [`wgpu`](https://crates.io/crates/wgpu) (DirectX 12 / Vulkan / Metal)             |
| Windowing      | [`winit`](https://crates.io/crates/winit)                                         |
| Asset Decoding | [`image`](https://crates.io/crates/image), [`tobj`](https://crates.io/crates/tobj)|
| Physics        | [`rapier3d`](https://crates.io/crates/rapier3d)                                   |
| UI / Debug     | [`egui`](https://crates.io/crates/egui) (planned)                                 |

## Development Goal

Sustain **200+ FPS** on target hardware (e.g., NVIDIA RTX 4060 Ti) in complex, heavily populated scenes, achieved through careful CPU/GPU balance, parallel ECS queries, and data‑oriented design principles.

## License

This project is licensed under either of [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
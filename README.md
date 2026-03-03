# RedOx Engine

High-performance, modular game engine written in Rust from scratch. Features a custom Archetype ECS and GPU-Driven rendering pipeline.

## Current Status

**Phase 0 Complete (Architecture & Foundation)** ✅

## Implemented Modules

- **Workspace configuration** with optimized build profiles (dev/release)
  - Release profile: LTO thin, single codegen unit, panic=abort
  - Dev profile: Optimized with per-package tuning
  
- **`redox_math`**: Fundamental math types and utilities
  - Vector types (`Vec2`, `Vec3`, `Vec4`)
  - Matrix types (`Mat4`) with transformation helpers
  - Quaternion support with axis-angle and spherical interpolation
  - Geometric bounds: Axis-Aligned Bounding Box (AABB) and Sphere
  - Frustum culling logic for view optimization

## Architecture

This project is organized as a Cargo workspace with the following structure:

```
redox_engine/
├── crates/
│   └── redox_math/      # Math library
├── Cargo.toml           # Workspace root
├── Cargo.lock
└── README.md
```

## Features

### Build Profiles

#### Release Profile
```toml
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"
```

#### Dev Profile
```toml
opt-level = 1
[profile.dev.package."*"]
opt-level = 3
```

### Dependencies

- **glam**: High-performance math library
- **wgpu**: Cross-platform graphics API
- **winit**: Window creation and event handling
- **rapier3d**: Physics engine (planned)
- **egui**: Immediate mode GUI (planned)
- **kira**: Audio system (planned)

## Roadmap

### Phase 1: ECS Core (Pending)
- Implement `redox_ecs` module
- Archetype-based entity component system
- System execution scheduling
- Resource management

### Phase 2: Rendering (Pending)
- GPU-driven rendering pipeline
- Material and shader system
- Render graph architecture

### Phase 3: Physics (Pending)
- Integration with rapier3d
- Collision detection and resolution
- Joint constraints

### Phase 4: Tools (Pending)
- Scene editor with egui
- Asset pipeline
- Debug tools

## Building

```bash
# Build in release mode
cargo build --release

# Run tests
cargo test

# Check for clippy warnings
cargo clippy --all-targets
```

## Contribution

This is a personal learning project focused on game engine development. Contributions, ideas, and feedback are welcome!

## License

MIT License - see LICENSE file for details.

---

*Built with ❤️ in Rust*

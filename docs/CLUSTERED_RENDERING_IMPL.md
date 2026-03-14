# Clustered Forward Rendering Implementation Guide

## Overview

This document describes the implementation of **Clustered Forward Rendering (CFR)** for the RedOx Engine. CFR dramatically improves performance when rendering scenes with many dynamic lights by spatially partitioning lights into 3D clusters, reducing per-fragment light loop iterations from O(num_lights) to O(lights_per_cluster).

## Current Implementation Status

### ✅ Completed Components

#### 1. **Clustering Module** (`crates/redox_render/src/clustering.rs`)
- **ClusterBounds**: Structure for storing min/max depth per cluster
- **ClusterMetadata**: Offset and count for light indices in each cluster
- **ClusterInfo**: GPU-friendly structure containing grid dimensions and depth configuration
- **build_clusters()**: Constructs logarithmic depth slices and cluster boundaries
- **assign_lights_to_clusters()**: CPU-based light-to-cluster assignment using sphere-frustum intersection
- **get_cluster_indices()**: Computes cluster (x,y,z) from screen coordinates and depth
- **get_depth_slice()**: Converts linear depth to cluster depth slice index
- **Logarithmic Depth Distribution**: Near clusters are thinner, far clusters are thicker (improves precision)

**Constants**:
- `CLUSTER_SIZE_X`: 16 pixels
- `CLUSTER_SIZE_Y`: 16 pixels
- `CLUSTER_DEPTH`: 24 depth slices
- `MAX_LIGHTS_PER_CLUSTER`: 256 lights
- `MAX_LIGHTS`: 512 total lights

#### 2. **Cluster Manager** (`crates/redox_render/src/cluster_manager.rs`)
- **ClusterManager**: Manages GPU cluster buffers and assignments
- **GPU Buffers**:
  - `metadata_buffer`: (offset, count) pairs for each cluster
  - `light_indices_buffer`: Flat array of light indices
  - `bounds_buffer`: Min/max depth for each cluster
  - `info_buffer`: Cluster grid configuration (uniform)
- **update_clusters()**: Updates assignments when lights or camera changes
- **rebuild_for_resolution()**: Resizes buffers on window resize
- **assignment_stats()**: Provides statistics for debugging (max lights per cluster, etc.)

#### 3. **Context Integration** (`crates/redox_render/src/context.rs`)
- Added `cluster_manager` field to `RenderContext`
- Added `update_cluster_lights()` method to update clusters with lights
- Added `cluster_manager()` getter for shader binding access
- Initialization in `new()` with dummy camera parameters

#### 4. **Light Structures** (`crates/redox_render/src/light.rs`)
- Added `PointLightGpu` struct for storage buffers
- Backward compatible with existing `LightUniform` for legacy code
- Can represent lights in both formats

#### 5. **Module Exports** (`crates/redox_render/src/lib.rs`)
- Exported `ClusterInfo`, `ClusterManager`, `PointLight`, `PointLightGpu`

### ⏳ Remaining Work

#### Phase 1: Shader Updates (High Priority)

**1.1 Update PBR Shader** (`crates/redox_render/src/shader/manager.rs`)

Replace the fixed light loop (lines 321-347) with cluster-based lookup:

```wgsl
// OLD:
for (var i = 0u; i < light_u.num_point_lights; i = i + 1u) {
    // evaluate all lights
}

// NEW:
let cluster_x = u32(in.clip_pos.x / 16.0);
let cluster_y = u32(in.clip_pos.y / 16.0);
let linear_depth = compute_linear_depth(in.clip_pos.z, camera);
let cluster_z = get_depth_slice(linear_depth);
let cluster_idx = cluster_x + cluster_y * clusters_x + cluster_z * clusters_x * clusters_y;

let metadata = cluster_light_offsets[cluster_idx];
for (var i = 0u; i < metadata.count; i = i + 1u) {
    let light_idx = cluster_light_indices[metadata.offset + i];
    let light = point_lights[light_idx];
    // evaluate individual light
}
```

**1.2 Add New Shader Bindings** (Group 0)

```wgsl
@group(0) @binding(10) var<storage, read> point_lights: array<PointLight>;
@group(0) @binding(11) var<storage, read> cluster_light_offsets: array<vec2<u32>>;
@group(0) @binding(12) var<storage, read> cluster_light_indices: array<u32>;
@group(0) @binding(13) var<uniform> cluster_info: ClusterInfo;
```

**1.3 Add Shader Structures**

```wgsl
struct PointLight {
    position: vec4<f32>,
    color: vec4<f32>,
    intensity: f32,
    radius: f32,
    _pad: vec2<f32>,
};

struct ClusterInfo {
    clusters_x: u32,
    clusters_y: u32,
    depth_slices: u32,
    screen_width: u32,
    screen_height: u32,
    near: f32,
    far: f32,
    depth_scale: f32,
};
```

**1.4 Add Helper Functions**

```wgsl
fn compute_linear_depth(ndc_depth: f32, near: f32, far: f32) -> f32 {
    // Convert NDC depth to linear depth
    return (2.0 * near * far) / ((far + near) - ndc_depth * (far - near));
}

fn get_depth_slice(linear_depth: f32, near: f32, depth_scale: f32, depth_slices: u32) -> u32 {
    let z_norm = ((linear_depth / near) |> ln() / depth_scale).max(0.0);
    return (z_norm as u32).min(depth_slices - 1);
}
```

#### Phase 2: Bind Group Layout Updates (High Priority)

**2.1 Update PBR Pass** (`crates/redox_render/src/pass/pbr.rs`)

Extend the global bind group layout (Group 0) to include:
- Binding 10: Storage buffer for point lights
- Binding 11: Storage buffer for cluster metadata
- Binding 12: Storage buffer for cluster light indices
- Binding 13: Uniform buffer for cluster info

**2.2 Update Global Bind Group Creation**

In `create_global_bind_group()`, add entries for the new cluster buffers:

```rust
wgpu::BindGroupEntry {
    binding: 10,
    resource: cluster_manager.point_lights_buffer.as_entire_binding(),
},
wgpu::BindGroupEntry {
    binding: 11,
    resource: cluster_manager.metadata_buffer.as_entire_binding(),
},
wgpu::BindGroupEntry {
    binding: 12,
    resource: cluster_manager.light_indices_buffer.as_entire_binding(),
},
wgpu::BindGroupEntry {
    binding: 13,
    resource: cluster_manager.info_buffer.as_entire_binding(),
},
```

#### Phase 3: Light Manager Integration (Medium Priority)

**3.1 Create LightManager** (Option A: New file or integrate into context)

```rust
pub struct LightManager {
    pub directional: DirectionalLight,
    pub point_lights: Vec<PointLight>,
    pub point_lights_gpu: Vec<PointLightGpu>,
    pub point_lights_buffer: wgpu::Buffer,
}

impl LightManager {
    pub fn new(device: &Device) -> Self { ... }
    pub fn update_lights(&mut self, lights: &[PointLight], queue: &Queue) { ... }
    pub fn update_gpu_buffers(&self, queue: &Queue) { ... }
}
```

**3.2 Integrate with RenderContext**

Replace direct `light_buffer` usage with `LightManager`:

```rust
pub struct RenderContext {
    pub light_manager: LightManager,
    pub light_uniform: LightUniform, // for directional light
    pub light_buffer: wgpu::Buffer,  // for directional light
    // ...
}
```

#### Phase 4: System Updates (Medium Priority)

**4.1 Update Render Systems** (`crates/redox_render/src/systems.rs`)

Hook into the light collection system to:
1. Collect all point lights from the scene
2. Convert to `PointLightGpu` format
3. Call `context.update_cluster_lights(lights, camera)`

**4.2 Update Examples**

- `simple_game.rs`: Ensure cluster updates when lights are created
- `horror_demo.rs`: Test with many lights to verify clustering benefits
- `benchmark.rs`: Add performance comparison (with/without clustering)

#### Phase 5: Point Light GPU Buffer (Medium Priority)

**5.1 Add Point Lights Buffer to ClusterManager**

```rust
pub struct ClusterManager {
    // ... existing fields ...
    pub point_lights_buffer: Buffer,  // NEW: Storage buffer for PointLightGpu
}
```

**5.2 Serialize Lights to GPU**

In `update_clusters()`, also serialize the point lights:

```rust
let gpu_lights: Vec<PointLightGpu> = lights
    .iter()
    .map(PointLightGpu::from_point_light)
    .collect();

let lights_bytes = bytemuck::cast_slice(&gpu_lights);
queue.write_buffer(&self.point_lights_buffer, 0, lights_bytes);
```

#### Phase 6: Compute Shader (Low Priority - Future Optimization)

**6.1 Implement Compute Clustering** (Optional for even better performance)

Create `clustering_compute.wgsl` for GPU-based light assignment:

```wgsl
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Compute cluster index from global invocation ID
    // For each light, check intersection and append to cluster list
    // Use atomic operations for thread-safe list updates
}
```

This would move light assignment from CPU to GPU, further improving performance on scenes with dynamic lights.

---

## Implementation Checklist

### Core Infrastructure (✅ DONE)
- [x] Clustering module with bounds and assignment
- [x] ClusterManager for GPU buffer management
- [x] RenderContext integration
- [x] Module exports

### Shader Updates (⏳ TODO)
- [ ] Add cluster bindings to global bind group layout
- [ ] Update PBR shader with cluster lookup
- [ ] Add depth linearization and cluster index computation
- [ ] Remove fixed light array from shader (or keep for backward compat)
- [ ] Add PointLight struct to shader

### System Integration (⏳ TODO)
- [ ] Create/update LightManager
- [ ] Hook systems to collect and update lights
- [ ] Add cluster update calls in render frame
- [ ] Update examples

### GPU Buffers (⏳ TODO)
- [ ] Create point_lights_buffer in ClusterManager
- [ ] Serialize PointLightGpu to GPU
- [ ] Bind in global bind group

### Testing & Optimization (⏳ TODO)
- [ ] Verify no regressions with existing scenes
- [ ] Test with varying light counts (10, 50, 100, 200+)
- [ ] Profile performance improvements
- [ ] Add debug visualization (optional: render cluster grid)

---

## Performance Expectations

**Current (Brute Force)**:
- Per-fragment cost: O(num_lights × light_eval_cost)
- 1080p, 128 lights: ~265 billion light evaluations/frame

**With Clustering** (conservative estimates):
- Avg lights per cluster: 10-20 (depends on scene)
- Per-fragment cost: O(lights_per_cluster × light_eval_cost)
- 1080p, 512 lights, 15 lights/cluster: ~311 million light evaluations/frame
- **Speedup: 850x in best case, 4-10x typical**

---

## Known Limitations & Future Improvements

1. **Spot Lights**: Not yet supported; would require extended cluster bounds
2. **Directional Lights**: Still infinite, treated separately (correct behavior)
3. **CPU Clustering**: Light assignment happens on CPU each frame; compute shader would offload to GPU
4. **Static Clusters**: Clusters rebuild each frame even if camera doesn't move; could optimize with dirty flags
5. **No Shadow Maps for Point Lights**: Shadow mapping only for directional light

---

## File Reference

| File | Changes | Status |
|------|---------|--------|
| `clustering.rs` | NEW | ✅ |
| `cluster_manager.rs` | NEW | ✅ |
| `context.rs` | Modified | ✅ |
| `lib.rs` | Modified | ✅ |
| `light.rs` | Modified (added PointLightGpu) | ✅ |
| `shader/manager.rs` | Needs shader updates | ⏳ |
| `pass/pbr.rs` | Needs bind group updates | ⏳ |
| `systems.rs` | Needs integration | ⏳ |
| Examples | Need testing | ⏳ |

---

## Code Examples

### Using the Cluster Manager

```rust
// In update systems:
let lights = collect_point_lights(&world);
let camera = get_active_camera(&world);

context.update_cluster_lights(&lights, &camera);

// Get statistics for debugging:
if let Some(stats) = context.cluster_manager().assignment_stats() {
    println!("Lights per cluster: avg={}, max={}", 
        stats.avg_lights_per_cluster, 
        stats.max_lights_in_cluster);
}
```

### Cluster Configuration

Adjustable constants in `clustering.rs::cluster_config`:

```rust
pub const CLUSTER_SIZE_X: u32 = 16;    // Smaller = more clusters = better culling but more overhead
pub const CLUSTER_SIZE_Y: u32 = 16;
pub const CLUSTER_DEPTH: u32 = 24;     // More slices = better depth precision
pub const MAX_LIGHTS_PER_CLUSTER: u32 = 256;  // Cap to prevent excessive work
```

---

## References

- **Clustered Forward Rendering**: [GPU-Driven Rendering Pipelines](https://www.gdcvault.com/play/1024612/Advanced-Rendering-with-DirectX-12)
- **Light Culling**: Standard technique in modern engines (Unreal Engine, Unity)
- **Depth Distribution**: Logarithmic distribution improves precision near camera


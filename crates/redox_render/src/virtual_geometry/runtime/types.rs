//! Public configuration, statistics and handle types of the VG runtime.

use super::*;

// ── Config & Stats ────────────────────────────────────────────────────────────

/// Configuration for [`VGSystem`].
#[derive(Clone, Debug)]
pub struct VGConfig {
    /// Maximum total vertices across all VG assets.
    pub max_vertices: usize,
    /// Maximum total indices across all VG assets.
    pub max_indices: usize,
    /// Maximum total meshlets across all VG assets.
    pub max_meshlets: usize,
    /// Maximum active instances.
    pub max_instances: usize,
    /// Maximum visible draw commands per frame.
    pub max_visible_commands: usize,
    /// LOD chain config used when building new VG assets.
    pub lod_config: LodChainConfig,
    /// When true, culling runs on GPU via `vg_cull.wgsl` (CPU fallback otherwise).
    pub gpu_cull: bool,
}

impl Default for VGConfig {
    fn default() -> Self {
        Self {
            max_vertices: 1 << 18,
            max_indices: 1 << 19,
            max_meshlets: MAX_GLOBAL_MESHLETS.min(1 << 16),
            max_instances: MAX_VG_INSTANCES,
            max_visible_commands: MAX_VISIBLE_COMMANDS,
            lod_config: LodChainConfig::default(),
            gpu_cull: false,
        }
    }
}

/// Per-frame statistics reported by [`VGSystem`].
#[derive(Clone, Debug, Default)]
pub struct VGStats {
    /// Meshlets before culling.
    pub total_meshlets: u32,
    /// Meshlets after culling.
    pub visible_meshlets: u32,
    /// Triangles before culling.
    pub total_triangles: u64,
    /// Triangles after culling.
    pub visible_triangles: u64,
    /// Registered virtual geometry assets.
    pub registered_assets: u32,
    /// Instances that are drawn.
    pub active_instances: u32,
}

// ── Asset handle ──────────────────────────────────────────────────────────────

/// Opaque identifier for a registered VG asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VGAssetId(pub u64);

/// Opaque handle for a spawned VG instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VGInstanceHandle(pub u32);

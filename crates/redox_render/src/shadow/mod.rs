//! Shadow subsystem: the shadow atlas
//! allocator, and the Virtual Shadow Map (VSM) stack — page tables,
//! clipmap state, visibility/depth passes, page caching and SMRT sampling.

pub mod atlas_allocator;
pub mod cache;
pub mod classify;
pub mod depth_pass;
pub mod page_request;
pub mod page_table;
pub mod shadow_atlas;
pub mod smrt;
pub mod vsm_clipmap;

pub use atlas_allocator::{
    ATLAS_PX, MAX_CASTERS, NO_TILE, ShadowAtlasAllocator, TILE_PX, TILES_PER_AXIS, TOTAL_TILES,
};
pub use classify::{
    SHADOW_MOVED_EPS, ShadowClass, ShadowPose, is_dynamic_caster, moved_shadow_bounds,
    next_shadow_poses,
};
pub use page_table::{PhysPageAllocator, VirtualPageTable};
pub use smrt::{SmrtConfig, SmrtQuality, SmrtUniform, smrt_sample_params};
pub use vsm_clipmap::{
    DirtyPage, VSM_MAX_LEVELS, VSM_PAGES_PER_SIDE, VsmClipmap, VsmConfig, VsmGpuInfo, VsmLevelGpu,
    VsmStats,
};

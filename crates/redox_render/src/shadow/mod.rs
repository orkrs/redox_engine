//! Shadow subsystem: cascaded shadow maps (CSM), the shadow atlas
//! allocator, and the Virtual Shadow Map (VSM) stack — page tables,
//! clipmap state, visibility/depth passes, page caching and SMRT sampling.

pub mod atlas_allocator;
pub mod cache;
pub mod csm;
pub mod depth_pass;
pub mod page_table;
pub mod shadow_atlas;
pub mod smrt;
pub mod vsm_clipmap;
pub mod visibility;

pub use atlas_allocator::{
    MAX_CASTERS, NO_TILE, ShadowAtlasAllocator, TILE_PX, TILES_PER_AXIS, TOTAL_TILES, ATLAS_PX,
};
pub use page_table::{PhysPageAllocator, VirtualPageTable};
pub use smrt::{SmrtConfig, SmrtQuality, SmrtUniform, smrt_sample_params};
pub use vsm_clipmap::{
    DirtyPage, VSM_MAX_LEVELS, VSM_PAGES_PER_SIDE, VsmClipmap, VsmConfig, VsmGpuInfo, VsmLevelGpu,
    VsmStats,
};

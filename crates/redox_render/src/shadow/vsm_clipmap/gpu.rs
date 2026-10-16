//! GPU-facing mirrors of the clipmap state (byte layouts shared with WGSL).

use super::VSM_MAX_LEVELS;

/// GPU mirror of one clipmap level — byte-compatible with
/// `shaders/vsm_visibility.wgsl::ClipmapLevel` (80 bytes; the final
/// `texel_world` occupies that shader's `_pad` slot).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VsmLevelGpu {
    pub view_proj: [[f32; 4]; 4],
    pub world_half_extent: f32,
    pub page_table_offset: u32,
    pub pages_per_side: u32,
    pub texel_world: f32,
}

/// Flat words per level in the combined page-table array.
/// Fixed at 256 (16×16 max) to keep `VsmGpuInfo` size stable (8896 B) for WGSL,
/// even when `VSM_PAGES_PER_SIDE` is reduced (e.g. 4) to cut mosaic.
pub const VSM_PAGE_TABLE_WORDS_PER_LEVEL: usize = 256;

/// Total words in [`VsmGpuInfo::page_tables`].
pub const VSM_PAGE_TABLE_WORDS: usize = VSM_MAX_LEVELS * VSM_PAGE_TABLE_WORDS_PER_LEVEL;
/// GPU mirror of `pbr.wgsl::VsmGpuInfo` (binding 24, storage read-only).
///
/// Layout contract (bytes): header 48 → `levels[8]` @ 48 (80 B each) →
/// `page_tables[2048]` @ 688. Do not reorder fields without updating the
/// WGSL side.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VsmGpuInfo {
    pub level_count: u32,
    pub page_px: u32,
    /// Physical pages in the pool.
    pub phys_page_count: u32,
    /// Atlas pages along one axis (√pool, rounded up).
    pub phys_pages_per_side: u32,
    /// SMRT params (mirror of [`SmrtUniform`](super::smrt::SmrtUniform)).
    pub ray_count: u32,
    pub samples_per_ray: u32,
    pub source_angle: f32,
    pub source_radius: f32,
    pub max_trace_dist: f32,
    pub _pad0: [f32; 3],
    /// 16-byte alignment pad: WGSL vec3<f32> has size 16, so the following
    /// array<..mat4..> must start at a 16-aligned offset (shader struct = 8896 B).
    pub _pad_align: [f32; 4],
    pub levels: [VsmLevelGpu; VSM_MAX_LEVELS],
    /// Concatenated page-table entries, `level * 256 + y * 16 + x`.
    pub page_tables: [u32; VSM_PAGE_TABLE_WORDS],
}

//! Configurable bounds and editor hooks for validation.

use redox_math::{Aabb, Vec3};
/// Default half-extent of the world AABB when none is configured (very wide).
const DEFAULT_HALF_EXTENT: f32 = 1_000_000.0;
/// Width or height above this counts as a "large" texture for the mip heuristic.
pub const DEFAULT_LARGE_TEXTURE: u32 = 2048;

/// Settings that control bounds checks and editor integration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidationSettings {
    /// Axis-aligned box that entities must stay inside.
    pub world_bounds: Aabb,
    /// Textures wider or taller than this trigger the mip heuristic.
    pub large_texture_threshold: u32,
    /// Log validation warnings when saving a level.
    pub warn_before_save: bool,
    /// Refuse play-in-editor while errors remain.
    pub block_pie_on_error: bool,
}

impl Default for ValidationSettings {
    fn default() -> Self {
        Self {
            world_bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::splat(DEFAULT_HALF_EXTENT)),
            large_texture_threshold: DEFAULT_LARGE_TEXTURE,
            warn_before_save: true,
            block_pie_on_error: true,
        }
    }
}

impl ValidationSettings {
    /// Copy with numeric fields clamped to sane ranges.
    pub fn clamped(self) -> Self {
        Self {
            large_texture_threshold: self.large_texture_threshold.clamp(256, 16_384),
            ..self
        }
    }
}

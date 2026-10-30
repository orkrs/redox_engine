//! GPU description of the two probe cascades, mirrored by `GiGridGpu` in `pbr.wgsl` and
//! `vg_resolve.wgsl` (`@group(1) @binding(16)`).
//!
//! Layout (WGSL uniform rules, 112 bytes): two `GiCascadeGpu` of 48 bytes each, then
//! `probes_per_row`, `enabled` and two words of padding.

use bytemuck::{Pod, Zeroable};

use super::cascade::ProbeCascade;

/// Number of probe cascades the GPU grid describes.
pub const GI_CASCADE_COUNT: usize = 2;

/// One cascade as the shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GiCascadeGpu {
    /// Reference origin (xyz) and probe spacing (w), world space.
    pub origin: [f32; 4],
    /// Probe counts (xyz) and the atlas index of the cascade's first probe (w).
    pub dims: [u32; 4],
    /// Lowest world cell of the window (xyz); w unused.
    pub scroll: [i32; 4],
}

/// Both cascades plus the atlas row length; the uniform at `@binding(16)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GiGridGpu {
    /// Fine cascade first, coarse second.
    pub cascades: [GiCascadeGpu; GI_CASCADE_COUNT],
    /// Probes per row of the irradiance and visibility atlases.
    pub probes_per_row: u32,
    /// Non-zero when GI lighting is applied.
    pub enabled: u32,
    /// Padding to a multiple of 16 bytes.
    pub _pad: [u32; 2],
}

impl GiGridGpu {
    /// Describes `cascades`; cascade `c` owns atlas probes `c * probe_count .. (c + 1) * probe_count`
    /// where `probe_count` is the probe count of the first cascade.
    ///
    /// ASSUMPTION: all cascades have the same probe count, so the atlas index of cascade `c`
    /// is `c * probe_count`; this holds for the 8×8×8 layout used by the renderer.
    pub fn new(cascades: &[ProbeCascade; GI_CASCADE_COUNT], probes_per_row: u32, enabled: bool) -> Self {
        let per_cascade = cascades[0].probe_count();
        let mut out = Self {
            probes_per_row,
            enabled: u32::from(enabled),
            ..Self::default()
        };
        for (c, cascade) in cascades.iter().enumerate() {
            out.cascades[c] = GiCascadeGpu {
                origin: [
                    cascade.origin.x,
                    cascade.origin.y,
                    cascade.origin.z,
                    cascade.spacing,
                ],
                dims: [
                    cascade.dims[0],
                    cascade.dims[1],
                    cascade.dims[2],
                    per_cascade * c as u32,
                ],
                scroll: [cascade.scroll[0], cascade.scroll[1], cascade.scroll[2], 0],
            };
        }
        out
    }
}

const _: () = assert!(std::mem::size_of::<GiCascadeGpu>() == 48);
const _: () = assert!(std::mem::size_of::<GiGridGpu>() == 112);

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;

    #[test]
    fn grid_uniform_is_112_bytes_and_16_aligned() {
        assert_eq!(std::mem::size_of::<GiGridGpu>(), 112);
        assert_eq!(std::mem::align_of::<GiGridGpu>(), 4);
        assert_eq!(std::mem::size_of::<GiGridGpu>() % 16, 0);
        assert_eq!(std::mem::size_of::<GiCascadeGpu>(), 48);
    }

    #[test]
    fn field_offsets_match_the_wgsl_mirror() {
        // WGSL: array<GiCascadeGpu, 2> starts at 0 with stride 48; the scalars follow at 96.
        assert_eq!(std::mem::offset_of!(GiGridGpu, cascades), 0);
        assert_eq!(std::mem::offset_of!(GiGridGpu, probes_per_row), 96);
        assert_eq!(std::mem::offset_of!(GiGridGpu, enabled), 100);
        assert_eq!(std::mem::offset_of!(GiCascadeGpu, origin), 0);
        assert_eq!(std::mem::offset_of!(GiCascadeGpu, dims), 16);
        assert_eq!(std::mem::offset_of!(GiCascadeGpu, scroll), 32);
    }

    #[test]
    fn the_second_cascade_starts_after_the_first_probes() {
        let fine = ProbeCascade::new(Vec3::ZERO, 0.5, [8, 8, 8]);
        let coarse = ProbeCascade::new(Vec3::ZERO, 2.0, [8, 8, 8]);
        let grid = GiGridGpu::new(&[fine, coarse], 32, true);
        assert_eq!(grid.cascades[0].dims[3], 0);
        assert_eq!(grid.cascades[1].dims[3], 512);
        assert_eq!(grid.cascades[1].origin[3], 2.0);
        assert_eq!(grid.enabled, 1);
    }

    #[test]
    fn scroll_is_copied_from_the_cascade_window() {
        let mut fine = ProbeCascade::new(Vec3::ZERO, 0.5, [8, 8, 8]);
        fine.snap_to_camera(Vec3::new(1.2, 0.0, 0.0));
        let coarse = ProbeCascade::new(Vec3::ZERO, 2.0, [8, 8, 8]);
        let grid = GiGridGpu::new(&[fine, coarse], 32, false);
        assert_eq!(grid.cascades[0].scroll[0], fine.scroll[0]);
        assert_eq!(grid.enabled, 0);
    }
}

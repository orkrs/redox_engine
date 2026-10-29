//! GPU layout of the reflection probe table read by the PBR shader.

use bytemuck::{Pod, Zeroable};

use super::probe::ReflectionProbe;
use super::set::MAX_PROBES;

/// One probe as seen by the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ProbeGpu {
    /// Capture centre in `xyz`, blend distance in `w`.
    pub center_blend: [f32; 4],
    /// Half extent of the influence box in `xyz`, priority (as float) in `w`.
    pub half_extent_prio: [f32; 4],
}

/// Probe table uniform: the number of valid probes followed by the probe array.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ProbesGpu {
    /// Number of valid entries at the start of `probes`.
    pub count: u32,
    /// Padding to a 16-byte boundary.
    pub pad: [u32; 3],
    /// Probe entries; entries past `count` are zero.
    pub probes: [ProbeGpu; MAX_PROBES],
}

impl ProbeGpu {
    /// Packs a probe into its GPU layout.
    pub fn from_probe(probe: &ReflectionProbe) -> Self {
        Self {
            center_blend: [
                probe.center.x,
                probe.center.y,
                probe.center.z,
                probe.blend_distance,
            ],
            half_extent_prio: [
                probe.half_extent.x,
                probe.half_extent.y,
                probe.half_extent.z,
                probe.priority as f32,
            ],
        }
    }
}

impl ProbesGpu {
    /// Builds the table from the occupied slots, in slot order.
    pub fn from_probes(probes: &[ReflectionProbe]) -> Self {
        let mut table = Self::default();
        for (entry, probe) in table.probes.iter_mut().zip(probes.iter().take(MAX_PROBES)) {
            *entry = ProbeGpu::from_probe(probe);
        }
        table.count = probes.len().min(MAX_PROBES) as u32;
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;

    #[test]
    fn probes_gpu_layout_is_16_aligned() {
        assert_eq!(std::mem::size_of::<ProbeGpu>(), 32);
        assert_eq!(std::mem::size_of::<ProbesGpu>(), 144);
        assert_eq!(std::mem::size_of::<ProbesGpu>() % 16, 0);
    }

    #[test]
    fn the_table_counts_only_the_given_probes() {
        let probe = ReflectionProbe {
            center: Vec3::new(1.0, 2.0, 3.0),
            half_extent: Vec3::splat(4.0),
            blend_distance: 0.5,
            priority: 2,
        };
        let table = ProbesGpu::from_probes(&[probe]);
        assert_eq!(table.count, 1);
        assert_eq!(table.probes[0].center_blend, [1.0, 2.0, 3.0, 0.5]);
        assert_eq!(table.probes[0].half_extent_prio, [4.0, 4.0, 4.0, 2.0]);
        assert_eq!(table.probes[1], ProbeGpu::default());
    }
}

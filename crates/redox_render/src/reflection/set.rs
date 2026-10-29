//! Reflection probe slots and the cube-array texture that stores their prefiltered captures.

use super::gpu::ProbesGpu;
use super::probe::ReflectionProbe;
use crate::memory_budget::texture_size_bytes;

/// Maximum number of probes that can be active at once.
pub const MAX_PROBES: usize = 4;
/// Edge length of each prefiltered probe face in texels.
pub const PROBE_FACE_SIZE: u32 = 128;
/// Mip levels of each prefiltered probe.
pub const PROBE_MIP_LEVELS: u32 = 5;
/// Format of the probe array (same as the global prefiltered map).
pub const PROBE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Cube faces per probe.
const FACES: u32 = 6;
/// Bytes per texel of [`PROBE_FORMAT`].
const PROBE_BYTES_PER_TEXEL: u32 = 8;

/// Keeps at most [`MAX_PROBES`] probes, highest priority first; the sort is stable so equal
/// priorities keep their input order.
pub fn pick_by_priority(probes: &[ReflectionProbe]) -> Vec<ReflectionProbe> {
    let mut sorted = probes.to_vec();
    sorted.sort_by(|a, b| b.priority.cmp(&a.priority));
    sorted.truncate(MAX_PROBES);
    sorted
}

/// Probe slots plus the cube-array texture holding one prefiltered cube per slot.
pub struct ReflectionProbeSet {
    probes: Vec<ReflectionProbe>,
    enabled: bool,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
}

impl ReflectionProbeSet {
    /// Creates an empty set with a zero-filled probe array on `device`.
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection_probe_array"),
            size: wgpu::Extent3d {
                width: PROBE_FACE_SIZE,
                height: PROBE_FACE_SIZE,
                depth_or_array_layers: FACES * MAX_PROBES as u32,
            },
            mip_level_count: PROBE_MIP_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PROBE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection_probe_array_view"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reflection_probe_table"),
            size: std::mem::size_of::<ProbesGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            probes: Vec::new(),
            enabled: false,
            texture,
            view,
            buffer,
        }
    }

    /// Whether the shader may use the probes. Disabled by default.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Enables or disables probe lookups; takes effect after [`Self::upload`].
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Replaces the active probes with the highest-priority ones (at most [`MAX_PROBES`]).
    /// Slot `i` is the probe at index `i`; its captured cube must be refreshed after a change.
    /// Returns the number of accepted probes.
    pub fn set_probes(&mut self, probes: &[ReflectionProbe]) -> usize {
        self.probes = pick_by_priority(probes);
        self.probes.len()
    }

    /// The active probes in slot order.
    pub fn probes(&self) -> &[ReflectionProbe] {
        &self.probes
    }

    /// The probe table the shader should see: the active probes when enabled, otherwise empty.
    pub fn table(&self) -> ProbesGpu {
        if self.enabled {
            ProbesGpu::from_probes(&self.probes)
        } else {
            ProbesGpu::default()
        }
    }

    /// Writes [`Self::table`] to the GPU buffer.
    pub fn upload(&self, queue: &wgpu::Queue) {
        self.write_table(queue, &self.table());
    }

    /// Writes an arbitrary table to the GPU buffer (the capture uses an empty one).
    pub fn write_table(&self, queue: &wgpu::Queue, table: &ProbesGpu) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(table));
    }

    /// The probe table uniform buffer (binding 9 of the frame group).
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Cube-array view over all slots (`CubeArray`, 6 layers per slot).
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The probe array texture (used as the copy target of captures).
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Approximate GPU bytes of the probe array including mips.
    pub fn gpu_bytes(&self) -> u64 {
        texture_size_bytes(
            PROBE_FACE_SIZE,
            PROBE_FACE_SIZE,
            PROBE_BYTES_PER_TEXEL,
            PROBE_MIP_LEVELS,
        ) * u64::from(FACES)
            * MAX_PROBES as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;

    fn probe(priority: i32) -> ReflectionProbe {
        ReflectionProbe {
            center: Vec3::ZERO,
            half_extent: Vec3::ONE,
            blend_distance: 0.0,
            priority,
        }
    }

    #[test]
    fn pick_keeps_the_four_highest_priorities() {
        let input: Vec<_> = [0, 5, 1, 4, 3, 2].into_iter().map(probe).collect();
        let kept: Vec<i32> = pick_by_priority(&input).iter().map(|p| p.priority).collect();
        assert_eq!(kept, vec![5, 4, 3, 2]);
    }
}

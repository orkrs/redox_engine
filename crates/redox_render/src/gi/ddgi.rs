//! DDGI (Dynamic Diffuse Global Illumination) probe grid — CPU scaffold.
//!
//! Maintains a regular 3-D grid of irradiance probes updated each frame.
//! GPU ray-tracing is **not** implemented; see [`crate::gi::ddgi_gpu`] for the
//! honest GPU sampling path (trilinear + octahedral from CPU-filled probes).

use redox_math::Vec3;

use super::ddgi_gpu::DDGI_OCT_SAMPLES;

/// Grid dimensions in probe cells (X × Y × Z).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridDimensions {
    /// Probe index along X.
    pub x: i32,
    /// Probe index along Y.
    pub y: i32,
    /// Probe index along Z.
    pub z: i32,
}

/// Single DDGI probe storing diffuse irradiance, octahedral samples, and validity.
#[derive(Clone, Copy, Debug)]
pub struct DdgiProbe {
    /// World-space position of the probe.
    pub position: Vec3,
    /// L0 / average irradiance (CPU convenience).
    pub irradiance: Vec3,
    /// How much of the probe's surroundings is valid (0..1).
    pub validity: f32,
    /// Octahedral irradiance samples (2×4) filled by [`crate::gi::DdgiGpu::update`].
    pub oct_samples: [Vec3; DDGI_OCT_SAMPLES],
}

impl Default for DdgiProbe {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            irradiance: Vec3::ZERO,
            validity: 0.0,
            oct_samples: [Vec3::ZERO; DDGI_OCT_SAMPLES],
        }
    }
}

/// Regular 3-D probe grid for DDGI.
#[derive(Clone, Debug)]
pub struct DdgiProbeGrid {
    /// World position of the first probe.
    pub origin: Vec3,
    /// Distance between neighboring probes.
    pub spacing: f32,
    /// Number of probes along each axis.
    pub dimensions: GridDimensions,
    /// All probes in grid order.
    pub probes: Vec<DdgiProbe>,
}

impl DdgiProbeGrid {
    /// Creates an empty probe grid with the given world-space layout.
    pub fn new(origin: Vec3, spacing: f32, dimensions: GridDimensions) -> Self {
        let count = (dimensions.x * dimensions.y * dimensions.z).max(0) as usize;
        let mut probes = Vec::with_capacity(count);
        for z in 0..dimensions.z {
            for y in 0..dimensions.y {
                for x in 0..dimensions.x {
                    let offset = Vec3::new(x as f32, y as f32, z as f32) * spacing;
                    probes.push(DdgiProbe {
                        position: origin + offset,
                        irradiance: Vec3::ZERO,
                        validity: 0.0,
                        oct_samples: [Vec3::ZERO; DDGI_OCT_SAMPLES],
                    });
                }
            }
        }
        Self {
            origin,
            spacing,
            dimensions,
            probes,
        }
    }

    /// Maps a linear probe index to grid coordinates.
    pub fn probe_coords(&self, index: usize) -> Option<GridDimensions> {
        let dx = self.dimensions.x as usize;
        let dy = self.dimensions.y as usize;
        let dz = self.dimensions.z as usize;
        if dx == 0 || dy == 0 || dz == 0 || index >= self.probes.len() {
            return None;
        }
        let z = index / (dx * dy);
        let rem = index % (dx * dy);
        let y = rem / dx;
        let x = rem % dx;
        Some(GridDimensions {
            x: x as i32,
            y: y as i32,
            z: z as i32,
        })
    }

    /// Linear index for grid coordinates, or `None` when out of range.
    pub fn probe_index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        if x < 0
            || y < 0
            || z < 0
            || x >= self.dimensions.x
            || y >= self.dimensions.y
            || z >= self.dimensions.z
        {
            return None;
        }
        let dx = self.dimensions.x as usize;
        let dy = self.dimensions.y as usize;
        Some((z as usize) * dx * dy + (y as usize) * dx + (x as usize))
    }

    /// Stub update: decays validity and leaves irradiance unchanged.
    pub fn update(&mut self, _delta_time: f32) {
        for probe in &mut self.probes {
            probe.validity = (probe.validity * 0.95).max(0.0);
        }
    }

    /// Samples irradiance at a world position via trilinear 8-probe blend.
    pub fn sample_irradiance(&self, world_pos: Vec3) -> Vec3 {
        if self.probes.is_empty() || self.spacing <= 0.0 {
            return Vec3::ZERO;
        }
        let local = (world_pos - self.origin) / self.spacing;
        let base = local.floor();
        let frac = local - base;
        let x0 = base.x as i32;
        let y0 = base.y as i32;
        let z0 = base.z as i32;

        let mut accum = Vec3::ZERO;
        let mut wsum = 0.0f32;
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    if let Some(idx) = self.probe_index(x0 + dx, y0 + dy, z0 + dz) {
                        let probe = &self.probes[idx];
                        let wx = if dx == 0 { 1.0 - frac.x } else { frac.x };
                        let wy = if dy == 0 { 1.0 - frac.y } else { frac.y };
                        let wz = if dz == 0 { 1.0 - frac.z } else { frac.z };
                        let w = wx * wy * wz * probe.validity;
                        accum += probe.irradiance * w;
                        wsum += w;
                    }
                }
            }
        }
        if wsum > 1e-5 {
            accum / wsum
        } else {
            Vec3::ZERO
        }
    }
}

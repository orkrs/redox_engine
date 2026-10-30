//! Global signed distance volume for GI (CPU side).
//!
//! One uniform voxel grid covers the static scene. Voxel `(x, y, z)` stores the signed
//! distance to the nearest surface at its centre `origin + (i + 0.5) * voxel`, clamped to
//! `±MAX_DISTANCE`, and the material of that surface:
//!
//! * `rgb` is the albedo;
//! * `a` is the emission scale: emitted radiance is `rgb * a * EMISSION_SCALE`.
//!
//! Boxes carry a full RGB emission. The volume stores it as `rgb = emission colour`,
//! `a = max(emission) / EMISSION_SCALE`, so emitters lose their own albedo (ASSUMPTION).
//! Object rotation is not representable by bricks; see `Renderer::set_gi_static_bricks`.

use crate::sdf::SdfBrick;
use redox_math::{Aabb, Vec3};

/// Distance clamp in metres; far voxels report this value (positive, "far outside").
pub const MAX_DISTANCE: f32 = 8.0;
/// Default volume resolution per axis (128³ voxels, 12.6 MB as R16Float + Rgba8Unorm).
pub const GLOBAL_SDF_DIMS: [u32; 3] = [128, 128, 128];
/// Emission scale stored in the alpha channel: `a = 1` means 16 radiance units.
pub const EMISSION_SCALE: f32 = 16.0;

/// Axis-aligned box with albedo and emission: the input of the analytic scene builders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SdfBox {
    /// Minimum corner in world space.
    pub min: Vec3,
    /// Maximum corner in world space.
    pub max: Vec3,
    /// Diffuse albedo (linear RGB).
    pub albedo: Vec3,
    /// Emitted radiance (linear RGB); zero for non-emissive boxes.
    pub emissive: Vec3,
}

/// Signed distance from `p` to the AABB `[min, max]` (negative inside, exact outside).
pub fn sdf_aabb(p: Vec3, min: Vec3, max: Vec3) -> f32 {
    let center = (min + max) * 0.5;
    let half = (max - min) * 0.5;
    let q = (p - center).abs() - half;
    q.max(Vec3::ZERO).length() + q.max_element().min(0.0)
}

/// Uniform voxel SDF with per-voxel material.
#[derive(Clone, Debug)]
pub struct GlobalSdf {
    /// World position of the volume's minimum corner.
    pub origin: Vec3,
    /// Edge length of one voxel in metres (cubic voxels).
    pub voxel: f32,
    /// Voxel counts per axis.
    pub dims: [u32; 3],
    /// Signed distance per voxel, x fastest, clamped to `±MAX_DISTANCE`.
    pub distance: Vec<f32>,
    /// Material per voxel: `rgb` albedo, `a` emission scale (see module docs).
    pub albedo: Vec<[u8; 4]>,
}

/// Best surface candidate seen so far for one voxel.
#[derive(Clone, Copy)]
struct Candidate {
    dist: f32,
    /// True when the voxel centre lies inside the brick's own domain (preferred).
    contained: bool,
    color: [f32; 4],
}

impl Candidate {
    const NONE: Self = Self {
        dist: MAX_DISTANCE,
        contained: false,
        color: [0.0; 4],
    };

    fn beats(&self, other: &Self) -> bool {
        (self.contained && !other.contained)
            || (self.contained == other.contained && self.dist < other.dist)
    }
}

impl GlobalSdf {
    /// Index of voxel `(x, y, z)` in the x-fastest layout used by `distance` and `albedo`.
    pub fn index(dims: [u32; 3], x: u32, y: u32, z: u32) -> usize {
        (x + y * dims[0] + z * dims[0] * dims[1]) as usize
    }

    /// World position of the centre of voxel `(x, y, z)`.
    pub fn voxel_centre(&self, x: u32, y: u32, z: u32) -> Vec3 {
        self.origin + (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)) * self.voxel
    }

    /// Builds the exact SDF of `boxes` over `bounds` (for tests, walls and simple rooms).
    ///
    /// `dims` are used as given; the voxel size is chosen so the volume covers `bounds`.
    pub fn from_boxes(boxes: &[SdfBox], bounds: Aabb, dims: [u32; 3]) -> Self {
        let mut sdf = Self::unset(bounds, dims);
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let p = sdf.voxel_centre(x, y, z);
                    let i = Self::index(dims, x, y, z);
                    let nearest = nearest_box(boxes, p);
                    sdf.distance[i] = nearest.0.clamp(-MAX_DISTANCE, MAX_DISTANCE);
                    sdf.albedo[i] = match nearest.1 {
                        Some(b) => encode_box_material(b.albedo, b.emissive),
                        None => [0; 4],
                    };
                }
            }
        }
        sdf
    }

    /// Builds the volume from baked bricks, each with its object colour `[r, g, b, emission]`.
    ///
    /// Inside a brick's domain the brick's field is used; outside it the distance to the
    /// domain is a lower bound. Voxels further than `MAX_DISTANCE` from every brick keep
    /// `MAX_DISTANCE`. Only voxels within `MAX_DISTANCE` of a brick are visited.
    pub fn from_bricks(bricks: &[(SdfBrick, [f32; 4])], bounds: Aabb, dims: [u32; 3]) -> Self {
        let mut sdf = Self::unset(bounds, dims);
        let n = sdf.distance.len();
        let mut best = vec![Candidate::NONE; n];
        for (brick, color) in bricks {
            let half = brick.half_size();
            let lo = brick.origin_ws - half - Vec3::splat(MAX_DISTANCE);
            let hi = brick.origin_ws + half + Vec3::splat(MAX_DISTANCE);
            let Some((min, max)) = sdf.voxel_range(lo, hi) else {
                continue;
            };
            for z in min[2]..=max[2] {
                for y in min[1]..=max[1] {
                    for x in min[0]..=max[0] {
                        let p = sdf.voxel_centre(x, y, z);
                        let cand = if brick.contains(p) {
                            Candidate {
                                dist: brick.sample_world(p),
                                contained: true,
                                color: *color,
                            }
                        } else {
                            Candidate {
                                dist: sdf_aabb(p, brick.origin_ws - half, brick.origin_ws + half),
                                contained: false,
                                color: *color,
                            }
                        };
                        let i = Self::index(dims, x, y, z);
                        if cand.beats(&best[i]) {
                            best[i] = cand;
                        }
                    }
                }
            }
        }
        for (i, cand) in best.iter().enumerate() {
            sdf.distance[i] = cand.dist.clamp(-MAX_DISTANCE, MAX_DISTANCE);
            let c = cand.color;
            sdf.albedo[i] = encode_rgba([c[0], c[1], c[2]], c[3]);
        }
        sdf
    }

    /// Trilinear distance at world position `p`, clamped to the border voxels.
    /// Mirrors the GPU sampler (`textureSampleLevel` with a linear clamp sampler).
    pub fn sample(&self, p: Vec3) -> f32 {
        let g = (p - self.origin) / self.voxel - Vec3::splat(0.5);
        let base = g.floor();
        let f = g - base;
        let [bx, by, bz] = base.to_array().map(|v| v as i32);
        let mut acc = 0.0;
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    let w = axis_weight(f.x, dx) * axis_weight(f.y, dy) * axis_weight(f.z, dz);
                    let i = self.clamped_index(bx + dx, by + dy, bz + dz);
                    acc += w * self.distance[i];
                }
            }
        }
        acc
    }

    /// Material of the voxel nearest to `p`, as `[r, g, b, a]` in `[0, 1]`.
    pub fn albedo_at(&self, p: Vec3) -> [f32; 4] {
        let c = ((p - self.origin) / self.voxel).floor().to_array();
        let i = self.clamped_index(c[0] as i32, c[1] as i32, c[2] as i32);
        self.albedo[i].map(|v| f32::from(v) / 255.0)
    }

    /// Distances encoded as IEEE half floats, ready for an `R16Float` upload.
    pub fn distance_f16_bits(&self) -> Vec<u16> {
        self.distance
            .iter()
            .map(|&d| half::f16::from_f32(d).to_bits())
            .collect()
    }

    /// Volume with the given bounds and resolution, every voxel at `MAX_DISTANCE`.
    fn unset(bounds: Aabb, dims: [u32; 3]) -> Self {
        let extent = (bounds.max - bounds.min).max(Vec3::splat(1e-3));
        let per_axis = extent / Vec3::from_array(dims.map(|d| d.max(1) as f32));
        let count = dims[0] as usize * dims[1] as usize * dims[2] as usize;
        Self {
            origin: bounds.min,
            voxel: per_axis.max_element(),
            dims,
            distance: vec![MAX_DISTANCE; count],
            albedo: vec![[0; 4]; count],
        }
    }

    /// Inclusive voxel index range covering `[lo, hi]`, or `None` when it misses the volume.
    fn voxel_range(&self, lo: Vec3, hi: Vec3) -> Option<([u32; 3], [u32; 3])> {
        let l = ((lo - self.origin) / self.voxel).floor().to_array();
        let h = ((hi - self.origin) / self.voxel).floor().to_array();
        let mut min = [0u32; 3];
        let mut max = [0u32; 3];
        for axis in 0..3 {
            let last = self.dims[axis] as f32 - 1.0;
            let start = l[axis].max(0.0);
            let end = h[axis].min(last);
            if start > end {
                return None;
            }
            min[axis] = start as u32;
            max[axis] = end as u32;
        }
        Some((min, max))
    }

    fn clamped_index(&self, x: i32, y: i32, z: i32) -> usize {
        let [dx, dy, dz] = self.dims.map(|d| d.max(1) as i32 - 1);
        Self::index(
            self.dims,
            x.clamp(0, dx) as u32,
            y.clamp(0, dy) as u32,
            z.clamp(0, dz) as u32,
        )
    }
}

/// Signed distance and box of the surface nearest to `p` (`None` for an empty scene).
fn nearest_box(boxes: &[SdfBox], p: Vec3) -> (f32, Option<&SdfBox>) {
    let mut best = (MAX_DISTANCE, None);
    for b in boxes {
        let d = sdf_aabb(p, b.min, b.max);
        if best.1.is_none() || d < best.0 {
            best = (d, Some(b));
        }
    }
    best
}

fn axis_weight(f: f32, offset: i32) -> f32 {
    if offset == 0 { 1.0 - f } else { f }
}

// ASSUMPTION: an emitter's own albedo is replaced by its emission colour, because the
// alpha-only emission scale cannot carry a full RGB emission next to an RGB albedo.
fn encode_box_material(albedo: Vec3, emission: Vec3) -> [u8; 4] {
    let max_e = emission.max_element();
    if max_e > 0.0 {
        let colour = emission / max_e;
        encode_rgba(colour.to_array(), (max_e / EMISSION_SCALE).min(1.0))
    } else {
        encode_rgba(albedo.to_array(), 0.0)
    }
}

fn encode_rgba(rgb: [f32; 3], a: f32) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(rgb[0]), q(rgb[1]), q(rgb[2]), q(a)]
}

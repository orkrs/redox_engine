//! Data of a foliage layer and its CPU reference culling.

use bytemuck::{Pod, Zeroable};

use crate::virtual_geometry::Frustum;

/// Most LODs a layer can hold; the GPU pass reserves this many slots.
pub const MAX_LODS: usize = 4;
/// Marks "no LOD": the instance is beyond the last LOD, or has no history yet.
pub const NO_LOD: u32 = u32::MAX;
/// Band around each LOD threshold, as a fraction of the distance, in which the previous LOD is kept.
pub const LOD_HYSTERESIS: f32 = 0.05;

/// One level of detail of a layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodRef {
    /// Mesh slot drawn at this LOD.
    pub mesh_slot: u32,
    /// Index count of the mesh at this LOD, written into the indirect draw.
    pub index_count: u32,
    /// Largest distance (m) at which this LOD is used; beyond the last LOD nothing is drawn.
    pub max_distance: f32,
}

/// One placement of a foliage model. The layout matches the GPU struct (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FoliageInstance {
    /// World position.
    pub pos: [f32; 3],
    /// Rotation around Y, in radians.
    pub yaw: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Per-instance random seed for shaders (colour, sway phase).
    pub seed: u32,
    /// Padding to the 32-byte WGSL stride.
    pub _pad: [u32; 2],
}

impl FoliageInstance {
    /// Creates an instance with zero padding.
    pub fn new(pos: [f32; 3], yaw: f32, scale: f32, seed: u32) -> Self {
        Self {
            pos,
            yaw,
            scale,
            seed,
            _pad: [0; 2],
        }
    }
}

/// A set of instances of one or more models sharing a material.
#[derive(Clone, Debug, PartialEq)]
pub struct FoliageLayer {
    /// LODs ordered by `max_distance`, nearest first.
    pub lods: Vec<LodRef>,
    /// Material slot shared by all LODs.
    pub material_slot: u32,
    /// Placements.
    pub instances: Vec<FoliageInstance>,
    /// Bounding radius of the model at unit scale; an instance's sphere has `scale * bound_radius`.
    pub bound_radius: f32,
}

impl FoliageLayer {
    /// Creates an empty layer. Fails when there are no LODs, too many, or their distances are not
    /// strictly increasing.
    pub fn new(
        lods: Vec<LodRef>,
        material_slot: u32,
        bound_radius: f32,
    ) -> Result<Self, &'static str> {
        if lods.is_empty() {
            return Err("a foliage layer needs at least one LOD");
        }
        if lods.len() > MAX_LODS {
            return Err("a foliage layer holds at most MAX_LODS LODs");
        }
        if lods
            .windows(2)
            .any(|w| w[1].max_distance <= w[0].max_distance)
        {
            return Err("LOD distances must increase strictly");
        }
        Ok(Self {
            lods,
            material_slot,
            instances: Vec::new(),
            bound_radius,
        })
    }
}

/// Camera for culling: the view-projection matrix (column-major) and the eye position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoliageView {
    /// View-projection matrix, column-major.
    pub view_proj: [[f32; 4]; 4],
    /// Eye position in world space.
    pub camera_pos: [f32; 3],
}

/// Pure LOD choice with hysteresis. `previous` is the LOD chosen last frame (or [`NO_LOD`]).
/// The previous LOD is kept while the distance stays inside its band; otherwise the nearest LOD
/// whose `max_distance` covers the distance is taken. Returns [`NO_LOD`] beyond the last LOD.
pub fn choose_lod(distance: f32, previous: u32, lods: &[LodRef]) -> u32 {
    if let Some(prev) = lods.get(previous as usize) {
        let far_ok = distance <= prev.max_distance * (1.0 + LOD_HYSTERESIS);
        let near_ok = match previous {
            0 => true,
            p => distance >= lods[p as usize - 1].max_distance * (1.0 - LOD_HYSTERESIS),
        };
        if far_ok && near_ok {
            return previous;
        }
    }
    lods.iter()
        .position(|lod| distance <= lod.max_distance)
        .map_or(NO_LOD, |i| i as u32)
}

/// Result of the CPU reference cull.
#[derive(Clone, Debug, PartialEq)]
pub struct CullOutcome {
    /// Visible `(lod, instance index)` pairs, ordered by instance index.
    pub draws: Vec<(u32, u32)>,
    /// LOD state of every instance after this frame; feed it back as `previous` next frame.
    pub lods: Vec<u32>,
}

/// CPU reference of the GPU cull: frustum test, then [`choose_lod`]. An instance outside the
/// frustum keeps its previous LOD state. `previous` must have one entry per instance.
pub fn cull_reference(layer: &FoliageLayer, view: &FoliageView, previous: &[u32]) -> CullOutcome {
    let planes = Frustum::from_view_proj(&view.view_proj).planes;
    let mut draws = Vec::new();
    let mut lods = Vec::with_capacity(layer.instances.len());
    for (index, inst) in layer.instances.iter().enumerate() {
        let prev = previous[index];
        let radius = inst.scale * layer.bound_radius;
        if !sphere_in_frustum(&planes, inst.pos, radius) {
            lods.push(prev);
            continue;
        }
        let lod = choose_lod(distance(inst.pos, view.camera_pos), prev, &layer.lods);
        lods.push(lod);
        if lod != NO_LOD {
            draws.push((lod, index as u32));
        }
    }
    CullOutcome { draws, lods }
}

fn sphere_in_frustum(planes: &[[f32; 4]; 6], center: [f32; 3], radius: f32) -> bool {
    planes.iter().all(|p| {
        let d = p[0] * center[0] + p[1] * center[1] + p[2] * center[2] + p[3];
        d >= -radius
    })
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

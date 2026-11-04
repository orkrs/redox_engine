//! Foliage layers: instance data, GPU culling with distance LOD, and one indirect draw per LOD
//! (FOL-03, step 1). Drawing the layer is a later step.

pub mod cull;
pub mod layer;

pub use cull::{FoliageCull, FoliageGpu};
pub use layer::{
    CullOutcome, FoliageInstance, FoliageLayer, FoliageView, LOD_HYSTERESIS, LodRef, MAX_LODS,
    NO_LOD, choose_lod, cull_reference,
};

//! Static lightmaps (LGT-01).

mod bake;
mod manifest;
mod uv;

pub use bake::{BAKE_RAYS, LightmapBakeError, LightmapBaker, LightmapImage, LightmapScene};
pub use manifest::{
    LightmapEntry, LightmapsManifest, read_lightmaps_manifest, write_lightmaps_manifest,
};
pub use uv::{
    DEFAULT_METERS_PER_TEXEL, MeshUv2Error, has_uv2, reject_mesh_without_uv2, uv_packer_available,
    uv2_in_unit_square,
};

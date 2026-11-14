//! Static lightmaps (LGT-01).

mod bake;
mod manifest;
mod uv;

pub use bake::{LightmapBakeError, LightmapBaker, LightmapImage, LightmapScene, BAKE_RAYS};
pub use manifest::{LightmapEntry, LightmapsManifest, read_lightmaps_manifest, write_lightmaps_manifest};
pub use uv::{
    DEFAULT_METERS_PER_TEXEL, MeshUv2Error, has_uv2, reject_mesh_without_uv2, uv2_in_unit_square,
    uv_packer_available,
};
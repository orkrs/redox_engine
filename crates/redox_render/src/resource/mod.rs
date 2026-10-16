//! GPU resource utilities — buffer and texture helpers.

pub mod blue_noise;
pub mod buffer;
pub mod ibl;
pub mod staging_ring;
pub mod texture;
pub mod toksvig;

pub use staging_ring::{StagingRing, DEFAULT_RING_CAPACITY, FRAMES_IN_FLIGHT};
pub use toksvig::{
    build_normal_mip_chain_toksvig, toksvig_roughness_delta, toksvig_variance,
};

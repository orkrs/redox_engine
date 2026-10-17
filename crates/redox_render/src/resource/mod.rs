//! GPU resource utilities — buffer and texture helpers.

pub mod blue_noise;
pub mod buffer;
/// Image-based lighting resources.
pub mod ibl;
pub mod staging_ring;
pub mod texture;
pub mod texture_stream;
pub mod toksvig;

pub use staging_ring::{DEFAULT_RING_CAPACITY, FRAMES_IN_FLIGHT, StagingRing};
pub use texture_stream::{StreamConfig, StreamStats, TextureStreamer};
pub use toksvig::{build_normal_mip_chain_toksvig, toksvig_roughness_delta, toksvig_variance};

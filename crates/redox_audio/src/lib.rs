//! Audio subsystem for the RedOx Engine.
//!
//! Provides cinematic 3D spatial audio via the `kira` audio engine with advanced
//! acoustic simulation. The primary ECS resource is [`AudioContext`], and spatial sources
//! are represented by the [`AudioEmitter`] and [`AudioListener`] components.
//!
//! Advanced features include:
//! - **Occlusion/Obstruction**: Raycasting to determine if sounds are blocked
//! - **Reverb Zones**: Define acoustic spaces that apply reverb when entered
//! - **Acoustic Materials**: Model how surfaces absorb/reflect sound
//! - **Preset Reverb**: Pre-configured reverb effects for common spaces
//!
//! Use [`Handle<AudioData>`] with the asset manager for async loading.

pub mod asset_types;
pub mod context;
pub mod components;
pub mod debug;
pub mod loader;
pub mod spatial;
pub mod systems;

pub use asset_types::AudioData;
pub use context::{sync_assets_to_audio, AudioContext};
pub use components::{
    AudioEmitter, AudioListener, SpatialAudioEmitter, ReverbZone, ReverbPreset, AcousticMaterial,
};
pub use loader::AudioLoader;
pub use debug::AudioDebugDraw;
pub use systems::{
    check_occlusion, reverb_listener_system, occlusion_raycast_system, compute_active_reverb,
    OcclusionResult,
};

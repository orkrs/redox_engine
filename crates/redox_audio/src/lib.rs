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
//! Use `Handle<AudioData>` with the asset manager for async loading.

pub mod asset_types;
pub mod components;
pub mod context;
pub mod debug;
pub mod loader;
pub mod mixer;
pub mod spatial;
pub mod systems;

pub use asset_types::AudioData;
pub use components::{
    AcousticMaterial, AudioEmitter, AudioListener, ReverbPreset, ReverbZone, SpatialAudioEmitter,
};
pub use context::{AudioContext, sync_assets_to_audio, with_audio};
pub use debug::AudioDebugDraw;
pub use loader::AudioLoader;
pub use mixer::{
    DuckingController, MixerBus, MixerBusId, MixerGraph, PlayMode, PlayRequest, db_to_linear,
    linear_to_db,
};
pub use systems::{
    OcclusionResult, check_occlusion, compute_active_reverb, occlusion_raycast_system,
    reverb_listener_system,
};

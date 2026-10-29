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

#![warn(missing_docs)]

pub mod asset_types;
pub mod components;
pub mod context;
pub mod cues;
pub mod debug;
pub mod loader;
pub mod mixer;
pub mod music;
mod emitter_voices;
mod music_adaptive;
mod music_apply;
pub mod music_layers;
mod music_play;
pub mod spatial;
pub mod systems;
pub mod voices;

pub use asset_types::AudioData;
pub use components::{
    AcousticMaterial, AudioEmitter, AudioListener, ReverbPreset, ReverbZone, SpatialAudioEmitter,
};
pub use context::{AudioContext, sync_assets_to_audio, with_audio};
pub use cues::{CueError, CueLibrary, CuePlay, CueRng, CueSelector, SoundCue, SplitMix64};
pub use debug::AudioDebugDraw;
pub use emitter_voices::{emitter_playback_system, emitter_target_volume};
pub use loader::AudioLoader;
pub use mixer::{
    DuckingController, MixerBus, MixerBusId, MixerGraph, PlayMode, PlayRequest, db_to_linear,
    linear_to_db,
};
pub use music::{MusicDirector, MusicFrame};
pub use music_layers::{AdaptiveMusic, MusicLayer};
pub use music_apply::{MusicVoice, MusicVoices};
pub use music_play::{KiraMusicVoice, MusicError};
pub use systems::{
    OcclusionResult, check_occlusion, compute_active_reverb, occlusion_from_hits,
    occlusion_raycast_system, reverb_listener_system,
};
pub use voices::{VoiceBudget, VoiceDecision};

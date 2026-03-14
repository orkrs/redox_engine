//! Audio subsystem for the RedOx Engine.
//!
//! Provides 3D spatial audio via the `kira` audio engine.
//! The primary ECS resource is [`AudioContext`], and spatial sources
//! are represented by the [`AudioEmitter`] and [`AudioListener`] components.
//! Use [`Handle<AudioData>`] with the asset manager for async loading.

pub mod asset_types;
pub mod context;
pub mod components;
pub mod loader;
pub mod spatial;

pub use asset_types::AudioData;
pub use context::{sync_assets_to_audio, AudioContext};
pub use components::{AudioEmitter, AudioListener};
pub use loader::AudioLoader;

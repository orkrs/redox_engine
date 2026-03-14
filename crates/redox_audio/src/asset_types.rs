//! Asset data type for audio used with [`redox_asset`].
//!
//! Raw audio file bytes; decoded to kira's `StaticSoundData` when synced to [`crate::context::AudioContext`].

/// Raw audio file bytes (e.g. WAV/OGG). Loaded via asset manager; decoded and cached
/// in [`crate::context::AudioContext`] when synced.
#[derive(Clone, Debug)]
pub struct AudioData(pub Vec<u8>);

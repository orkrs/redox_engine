//! Audio asset loader for [`AudioData`].
//!
//! Register with [`redox_asset::AssetManager::register_loader`] to support
//! `load_async::<AudioData>(path)`. Raw bytes are stored; decoding happens
//! when synced to [`crate::context::AudioContext`].

use redox_asset::loader::AssetLoader;
use redox_asset::AssetError;

use crate::asset_types::AudioData;

/// Loads audio files as raw bytes ([`AudioData`]). Supported formats: WAV, OGG.
pub struct AudioLoader;

impl AssetLoader for AudioLoader {
    type Asset = AudioData;

    fn load(&self, bytes: &[u8]) -> Result<Self::Asset, AssetError> {
        Ok(AudioData(bytes.to_vec()))
    }

    fn extensions(&self) -> Vec<&'static str> {
        vec!["wav", "ogg"]
    }
}

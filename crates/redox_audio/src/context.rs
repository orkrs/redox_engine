//! Audio context — ECS resource wrapping kira's AudioManager.

use std::collections::HashMap;
use std::io::Cursor;

use kira::manager::{AudioManager, AudioManagerSettings};
use kira::manager::backend::DefaultBackend;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::tween::Tween;
use redox_asset::{AssetId, Handle};

use crate::asset_types::AudioData;

/// ECS resource that manages the audio engine.
///
/// Wraps `kira::AudioManager` and provides simple play/volume controls.
/// When using the asset system, call [`Self::add_sound_from_asset`] when an
/// [`AudioData`] is loaded, then [`Self::play_sound_by_handle`] to play.
pub struct AudioContext {
    manager: Option<AudioManager>,
    /// Master volume (0.0 → 1.0).
    pub master_volume: f64,
    /// Cached decoded sounds by asset handle id (from [`Handle<AudioData>`]).
    sound_cache: HashMap<AssetId, StaticSoundData>,
}

impl AudioContext {
    /// Initialises the audio context.
    ///
    /// If the audio backend fails (e.g., no audio device), the context is
    /// created in a degraded mode where all operations are no-ops.
    pub fn new() -> Self {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| {
                log::warn!("Audio backend unavailable: {}. Running without audio.", e);
                e
            })
            .ok();
        Self {
            manager,
            master_volume: 1.0,
            sound_cache: HashMap::new(),
        }
    }

    /// Returns `true` if the audio backend is available.
    pub fn is_available(&self) -> bool {
        self.manager.is_some()
    }

    /// Returns a mutable reference to the internal kira `AudioManager`, if available.
    pub fn manager_mut(&mut self) -> Option<&mut AudioManager> {
        self.manager.as_mut()
    }

    /// Sets the master volume (clamped to 0.0–1.0).
    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        if let Some(m) = &mut self.manager {
            let _ = m.main_track().set_volume(self.master_volume, Tween::default());
        }
    }

    /// Plays a static sound and returns a handle to the playing sound.
    pub fn play_sound(&mut self, data: StaticSoundData) -> Option<StaticSoundHandle> {
        self.manager.as_mut().and_then(|m| m.play(data).ok())
    }

    /// Plays a static sound with spatial settings.
    pub fn play_spatial(&mut self, data: StaticSoundData, _position: redox_math::Vec3, _max_distance: f32) -> Option<StaticSoundHandle> {
        // Simplified for now: just plays it.
        // In a fuller version we'd use kira's panning/volume based on listener.
        let settings = StaticSoundSettings::new();
        let mut data = data;
        data.settings = settings;

        self.play_sound(data)
    }

    /// Applies a low-pass filter to the main track (for occlusion/obstruction effect).
    /// Filter cutoff frequency in Hz (20000.0 = no filtering, lower = more muffled).
    pub fn set_lowpass_filter(&mut self, cutoff_hz: f32) {
        // Note: Full kira integration would apply this via effect chains
        // This is a placeholder for the architectural approach
        if let Some(_m) = &mut self.manager {
            // In a full implementation:
            // m.main_track().set_effect(lowpass_effect);
            log::debug!("Setting lowpass filter to {:.0} Hz", cutoff_hz);
        }
    }

    /// Applies reverb settings to the main track.
    /// This would typically be called from the reverb system after zone blending.
    pub fn set_reverb(&mut self, _decay: f32, _damping: f32) {
        // Note: kira 0.8+ has limited reverb support out of the box
        // For full cinematic reverb, consider: convolver effects, multiple delay lines, or external libraries
        if let Some(_m) = &mut self.manager {
            // In a full implementation:
            // m.main_track().set_effect(reverb_effect);
            log::debug!("Setting reverb with decay {:.2}s, damping {:.2}", _decay, _damping);
        }
    }

    /// Updates spatial parameters for an emitter (volume, pan) based on listener position.
    pub fn update_spatial_parameters(
        &self,
        emitter_pos: redox_math::Vec3,
        listener_pos: redox_math::Vec3,
        listener_forward: redox_math::Vec3,
        listener_up: redox_math::Vec3,
        max_distance: f32,
    ) -> (f32, f32) {
        let to_emitter = emitter_pos - listener_pos;
        let distance = to_emitter.length();

        // Calculate volume attenuation
        let volume = if distance >= max_distance {
            0.0
        } else if distance < 0.1 {
            1.0
        } else {
            ((max_distance - distance) / max_distance).clamp(0.0, 1.0)
        };

        // Calculate pan (left/right) based on cross product with listener forward
        let listener_right = listener_forward.cross(listener_up).normalize();
        let pan = to_emitter.normalize().dot(listener_right).clamp(-1.0, 1.0);

        (volume, pan)
    }

    /// Caches decoded sound from asset data and associates it with the handle.
    /// No-op if this handle is already cached. Returns `true` if the sound was added.
    pub fn add_sound_from_asset(&mut self, handle: Handle<AudioData>, data: &[u8]) -> bool {
        if self.sound_cache.contains_key(&handle.id()) {
            return false;
        }
        let cursor = Cursor::new(data.to_vec());
        let settings = StaticSoundSettings::new();
        match StaticSoundData::from_cursor(cursor, settings) {
            Ok(sound) => {
                self.sound_cache.insert(handle.id(), sound);
                true
            }
            Err(e) => {
                log::warn!("Failed to decode audio asset {:?}: {:?}", handle.id(), e);
                false
            }
        }
    }

    /// Returns whether the given handle has been synced (decoded and cached).
    pub fn has_sound(&self, handle: Handle<AudioData>) -> bool {
        self.sound_cache.contains_key(&handle.id())
    }

    /// Plays a cached sound by handle. Returns a handle to the playing sound if the
    /// asset was synced and the backend is available.
    pub fn play_sound_by_handle(&mut self, handle: Handle<AudioData>) -> Option<StaticSoundHandle> {
        let data = self.sound_cache.get(&handle.id())?.clone();
        self.play_sound(data)
    }

    /// Plays a cached sound by handle with spatial settings.
    pub fn play_spatial_by_handle(
        &mut self,
        handle: Handle<AudioData>,
        position: redox_math::Vec3,
        max_distance: f32,
    ) -> Option<StaticSoundHandle> {
        let data = self.sound_cache.get(&handle.id())?.clone();
        self.play_spatial(data, position, max_distance)
    }
}

/// Syncs loaded [`AudioData`] assets from the asset manager into the audio context.
/// Call each frame after `asset_manager.update(world)`. For each handle in the slice,
/// if the asset is ready and not yet cached, decodes and caches it.
pub fn sync_assets_to_audio(
    audio_ctx: &mut AudioContext,
    asset_manager: &redox_asset::AssetManager,
    audio_handles: &[Handle<AudioData>],
) {
    for &handle in audio_handles {
        if audio_ctx.has_sound(handle) {
            continue;
        }
        if let Some(data) = asset_manager.get(handle) {
            audio_ctx.add_sound_from_asset(handle, &data.0);
        }
    }
}

impl Default for AudioContext {
    fn default() -> Self {
        Self::new()
    }
}

// Note: AudioManager is not Send+Sync by default in all kira backends.
// We wrap it in Option to handle the case where the backend doesn't initialize.
// For ECS resource storage we need Send + Sync, which AudioManager<DefaultBackend>
// provides on most platforms.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_creation_does_not_panic() {
        // This may or may not have an audio device in CI, so we just verify
        // that creation doesn't panic.
        let ctx = AudioContext::new();
        // In a no-audio environment, manager will be None.
        let _ = ctx.is_available();
    }

    #[test]
    fn master_volume_clamped() {
        let mut ctx = AudioContext::new();
        ctx.set_master_volume(2.0);
        assert!((ctx.master_volume - 1.0).abs() < 0.001);
        ctx.set_master_volume(-0.5);
        assert!((ctx.master_volume - 0.0).abs() < 0.001);
    }
}

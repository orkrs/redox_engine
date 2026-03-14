//! Audio ECS components.

use redox_asset::Handle;
use redox_math::Vec3;

use crate::asset_types::AudioData;

/// An audio source attached to an entity.
///
/// Uses a handle to [`AudioData`] loaded via the asset manager. When the asset
/// is loaded and synced to [`crate::context::AudioContext`], playback can use
/// [`AudioContext::play_sound_by_handle`] or [`AudioContext::play_spatial_by_handle`].
#[derive(Debug, Clone)]
pub struct AudioEmitter {
    /// Handle to the audio asset (WAV/OGG bytes). Playback uses the context's cached sound.
    pub sound_handle: Option<Handle<AudioData>>,
    /// World-space position of the source.
    pub position: Vec3,
    /// Volume multiplier (1.0 = full volume).
    pub volume: f32,
    /// Pitch multiplier (1.0 = normal pitch).
    pub pitch: f32,
    /// Whether the sound should loop.
    pub looping: bool,
    /// Whether the sound is currently playing.
    pub playing: bool,
    /// Maximum audible distance.
    pub max_distance: f32,
}

impl AudioEmitter {
    /// Creates a new emitter with no sound (sound_handle = None).
    pub fn new(position: Vec3) -> Self {
        Self {
            sound_handle: None,
            position,
            volume: 1.0,
            pitch: 1.0,
            looping: false,
            playing: false,
            max_distance: 50.0,
        }
    }

    /// Sets the sound to play via asset handle.
    pub fn with_sound(mut self, handle: Handle<AudioData>) -> Self {
        self.sound_handle = Some(handle);
        self
    }

    /// Convenience: starts playing.
    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Convenience: stops playing.
    pub fn stop(&mut self) {
        self.playing = false;
    }
}

impl Default for AudioEmitter {
    fn default() -> Self {
        Self::new(Vec3::ZERO)
    }
}

/// The audio listener (usually attached to the camera entity).
///
/// There should be exactly one listener in the world.
#[derive(Debug, Clone)]
pub struct AudioListener {
    /// World-space position.
    pub position: Vec3,
    /// Forward direction (normalized).
    pub forward: Vec3,
    /// Up direction (normalized).
    pub up: Vec3,
}

impl AudioListener {
    pub fn new(position: Vec3, forward: Vec3, up: Vec3) -> Self {
        Self {
            position,
            forward: forward.normalize(),
            up: up.normalize(),
        }
    }
}

impl Default for AudioListener {
    fn default() -> Self {
        Self::new(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitter_defaults() {
        let e = AudioEmitter::new(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(e.volume, 1.0);
        assert!(!e.playing);
        assert!(!e.looping);
        assert!(e.sound_handle.is_none());
    }

    #[test]
    fn emitter_play_stop() {
        let mut e = AudioEmitter::default();
        e.play();
        assert!(e.playing);
        e.stop();
        assert!(!e.playing);
    }

    #[test]
    fn listener_defaults() {
        let l = AudioListener::default();
        assert!((l.forward - Vec3::NEG_Z).length() < 0.001);
        assert!((l.up - Vec3::Y).length() < 0.001);
    }
}

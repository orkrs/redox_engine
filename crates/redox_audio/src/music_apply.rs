//! Applies [`MusicFrame`]s to playing music tracks (plan item B8, music).
//!
//! The director produces gains and finished names; this module turns them into calls on
//! [`MusicVoice`] implementations. It has no audio device, so it is tested with fakes. The kira
//! implementation of [`MusicVoice`] comes later.

use std::collections::HashMap;

use crate::music::MusicFrame;

/// One playing music track.
pub trait MusicVoice {
    /// Sets the linear volume, already clamped to `[0, 1]`.
    fn set_volume(&mut self, gain: f32);
    /// Stops the track for good.
    fn stop(&mut self);
}

/// The playing tracks by name.
pub struct MusicVoices<V: MusicVoice> {
    voices: HashMap<String, V>,
}

impl<V: MusicVoice> Default for MusicVoices<V> {
    fn default() -> Self {
        Self {
            voices: HashMap::new(),
        }
    }
}

impl<V: MusicVoice> MusicVoices<V> {
    /// Creates an empty set of voices.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a voice under `name`. An existing voice of that name is stopped and replaced.
    pub fn insert(&mut self, name: impl Into<String>, voice: V) {
        if let Some(mut old) = self.voices.insert(name.into(), voice) {
            old.stop();
        }
    }

    /// True when a voice is registered under `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.voices.contains_key(name)
    }

    /// The number of registered voices.
    pub fn len(&self) -> usize {
        self.voices.len()
    }

    /// True when no voice is registered.
    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    /// Applies one frame.
    ///
    /// For every `(name, gain)` of the frame that has a voice, the volume is set to the gain
    /// clamped to `[0, 1]` (NaN counts as 0). For every name in `frame.finished`, the voice is
    /// stopped and removed. Names without a voice are ignored, and voices not mentioned in the
    /// frame are left alone.
    pub fn apply(&mut self, frame: &MusicFrame) {
        for (name, gain) in &frame.gains {
            if let Some(voice) = self.voices.get_mut(name) {
                voice.set_volume(sanitize_gain(*gain));
            }
        }
        for name in &frame.finished {
            if let Some(mut voice) = self.voices.remove(name) {
                voice.stop();
            }
        }
    }
}

/// Clamps a gain to `[0, 1]`, mapping NaN to silence.
fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_nan() {
        0.0
    } else {
        gain.clamp(0.0, 1.0)
    }
}

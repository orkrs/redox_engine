//! Playback of [`AdaptiveMusic`] on [`AudioContext`].
//!
//! All layers start together as silent looping streams, one kira voice per layer, stored in the
//! same map as the crossfaded tracks. [`AudioContext::update_music`] moves each layer's gain toward
//! the value the control asks for, at [`LAYER_FADE_SECONDS`] from silence to full.
//!
//! Adaptive music and the crossfade director do not mix: [`AudioContext::play_music`] fades the
//! adaptive music out, and [`AudioContext::play_adaptive_music`] replaces the layers at once.

use crate::context::AudioContext;
use crate::music::MusicFrame;
use crate::music_layers::{AdaptiveMusic, approach};

/// Seconds a layer needs to go from silence to full (and back).
pub(crate) const LAYER_FADE_SECONDS: f32 = 1.5;

/// The adaptive music being played: its layers, the control value and the smoothed gains.
pub(crate) struct AdaptiveState {
    music: AdaptiveMusic,
    control: f32,
    current_gains: Vec<f32>,
    /// Set by `stop_adaptive_music`: every layer fades to silence, then its voice is removed.
    fading: bool,
}

impl AudioContext {
    /// Starts every layer of `music` as a silent looping stream, all in the same call so that they
    /// begin together. Layers whose track is not registered are skipped with a warning. Layers of
    /// an adaptive track that is already playing are removed at once.
    pub fn play_adaptive_music(&mut self, music: AdaptiveMusic) {
        self.clear_adaptive_voices();
        for layer in &music.layers {
            if !self.has_music(&layer.track) {
                log::warn!(
                    "adaptive music layer `{}` is not a registered track; skipped",
                    layer.track
                );
                continue;
            }
            if self.music_voices.contains(&layer.track) {
                continue;
            }
            if let Some(voice) = self.open_music_voice(&layer.track) {
                self.music_voices.insert(layer.track.clone(), voice);
            }
        }
        let layer_count = music.layers.len();
        self.adaptive = Some(AdaptiveState {
            music,
            control: 0.0,
            current_gains: vec![0.0; layer_count],
            fading: false,
        });
    }

    /// Sets the control value (0..1; NaN counts as 0). The layer gains follow it smoothly, about
    /// 1.5 s from silence to full, on each [`AudioContext::update_music`]. No effect without
    /// adaptive music.
    pub fn set_music_control(&mut self, control: f32) {
        let control = if control.is_nan() {
            0.0
        } else {
            control.clamp(0.0, 1.0)
        };
        if let Some(state) = self.adaptive.as_mut() {
            state.control = control;
        }
    }

    /// Stops the adaptive music with a fade: the layers go silent over about 1.5 s, then their
    /// voices are removed on `update_music`.
    pub fn stop_adaptive_music(&mut self) {
        if let Some(state) = self.adaptive.as_mut() {
            state.fading = true;
        }
    }

    /// Removes the voices of the current adaptive layers at once.
    fn clear_adaptive_voices(&mut self) {
        let Some(old) = self.adaptive.take() else {
            return;
        };
        let finished: Vec<String> = old
            .music
            .layers
            .into_iter()
            .map(|layer| layer.track)
            .collect();
        self.music_voices.apply(&MusicFrame {
            gains: Vec::new(),
            finished,
        });
    }

    /// Moves the layer gains toward their targets by `dt` seconds and applies them to the voices.
    /// Ends the adaptive music once every layer has faded out after a stop.
    pub(crate) fn update_adaptive_music(&mut self, dt: f32) {
        let Some(state) = self.adaptive.as_mut() else {
            return;
        };
        let rate = 1.0 / LAYER_FADE_SECONDS;
        let targets = state.music.gains(state.control);
        let mut gains = Vec::with_capacity(targets.len());
        let mut finished = Vec::new();
        for (index, layer) in state.music.layers.iter().enumerate() {
            let target = if state.fading { 0.0 } else { targets[index] };
            let current = approach(state.current_gains[index], target, rate, dt);
            state.current_gains[index] = current;
            if state.fading && current <= 0.0 {
                finished.push(layer.track.clone());
            } else {
                gains.push((layer.track.clone(), current));
            }
        }
        let done = state.fading && state.current_gains.iter().all(|gain| *gain <= 0.0);
        self.music_voices.apply(&MusicFrame { gains, finished });
        if done {
            self.adaptive = None;
        }
    }
}

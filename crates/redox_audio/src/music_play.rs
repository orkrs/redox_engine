//! Music playback control on [`AudioContext`].
//!
//! The crossfade logic lives in [`MusicDirector`](crate::music::MusicDirector). This module only
//! exposes it on the context. It works without an audio device: the caller applies the gains from
//! each [`MusicFrame`] to its own players.

use crate::context::AudioContext;
use crate::music::{MusicDirector, MusicFrame};

/// Crossfade time in seconds used until [`AudioContext::set_music_crossfade`] is called.
pub(crate) const DEFAULT_MUSIC_CROSSFADE_SECONDS: f32 = 2.0;

impl AudioContext {
    /// Sets the crossfade time for music changes. Default 2.0 s.
    ///
    /// The director fixes its crossfade at construction, so it is rebuilt here. The current track,
    /// if any, is replayed on the new director. Tracks that were still fading out are dropped.
    /// A negative or NaN value switches instantly.
    pub fn set_music_crossfade(&mut self, seconds: f32) {
        let current = self.music.current().map(str::to_owned);
        self.music = MusicDirector::new(seconds);
        if let Some(track) = current {
            self.music.play(&track);
        }
    }

    /// Starts `track` fading in and fades the other tracks out. Playing the current track again
    /// does nothing.
    pub fn play_music(&mut self, track: &str) {
        self.music.play(track);
    }

    /// Fades all music out.
    pub fn stop_music(&mut self) {
        self.music.stop();
    }

    /// The selected music track, if any.
    pub fn current_music(&self) -> Option<&str> {
        self.music.current()
    }

    /// Advances the music fades by `dt` seconds and returns the frame: the gains of the audible
    /// tracks and the tracks that went silent during this update.
    pub fn update_music(&mut self, dt: f32) -> MusicFrame {
        self.music.update(dt)
    }
}

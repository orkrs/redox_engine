//! Music playback control on [`AudioContext`].
//!
//! The crossfade logic lives in [`MusicDirector`](crate::music::MusicDirector). Registered tracks
//! (encoded Ogg / MP3 / FLAC / WAV bytes) are streamed from memory through kira in a loop, one
//! voice per track, and [`AudioContext::update_music`] applies the crossfade gains to those voices
//! ([`MusicVoices`](crate::music_apply::MusicVoices)). Without an audio device everything still
//! works as pure logic: no voice is started and the frames are returned as before.

use std::io::Cursor;
use std::sync::Arc;

use kira::Tween;
use kira::sound::FromFileError;
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use thiserror::Error;

use crate::context::{AudioContext, gain_to_db};
use crate::music::{MusicDirector, MusicFrame};
use crate::music_apply::MusicVoice;

/// Crossfade time in seconds used until [`AudioContext::set_music_crossfade`] is called.
pub(crate) const DEFAULT_MUSIC_CROSSFADE_SECONDS: f32 = 2.0;

/// Why a music file could not be registered.
#[derive(Debug, Error)]
pub enum MusicError {
    /// The bytes are not a supported audio file.
    #[error("music file could not be decoded: {0}")]
    Decode(String),
}

/// A streaming kira sound used as a music voice.
pub struct KiraMusicVoice {
    handle: StreamingSoundHandle<FromFileError>,
}

impl MusicVoice for KiraMusicVoice {
    fn set_volume(&mut self, gain: f32) {
        self.handle.set_volume(gain_to_db(gain), Tween::default());
    }

    fn stop(&mut self) {
        self.handle.stop(Tween::default());
    }
}

fn open_stream(bytes: &Arc<[u8]>) -> Result<StreamingSoundData<FromFileError>, FromFileError> {
    StreamingSoundData::from_cursor(Cursor::new(Arc::clone(bytes)))
}

impl AudioContext {
    /// Registers the encoded bytes of a music file under `name` (replacing an earlier one).
    ///
    /// The header is decoded once to reject bad data; the file is streamed from memory when the
    /// track plays.
    pub fn register_music(
        &mut self,
        name: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<(), MusicError> {
        let bytes: Arc<[u8]> = bytes.into();
        open_stream(&bytes).map_err(|e| MusicError::Decode(e.to_string()))?;
        self.music_sources.insert(name.into(), bytes);
        Ok(())
    }

    /// Whether a track of that name is registered.
    pub fn has_music(&self, name: &str) -> bool {
        self.music_sources.contains_key(name)
    }

    /// Number of streaming voices that are alive (playing or fading out).
    pub fn playing_music_voices(&self) -> usize {
        self.music_voices.len()
    }

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
    /// does nothing. With a device and a registered file the track starts streaming (silent until
    /// [`Self::update_music`] raises it).
    pub fn play_music(&mut self, track: &str) {
        self.music.play(track);
        self.start_music_voice(track);
    }

    /// Fades all music out.
    pub fn stop_music(&mut self) {
        self.music.stop();
    }

    /// The selected music track, if any.
    pub fn current_music(&self) -> Option<&str> {
        self.music.current()
    }

    /// Advances the music fades by `dt` seconds, applies the gains to the streaming voices and
    /// returns the frame: the gains of the audible tracks and the tracks that went silent.
    pub fn update_music(&mut self, dt: f32) -> MusicFrame {
        let frame = self.music.update(dt);
        self.music_voices.apply(&frame);
        frame
    }

    /// Starts the streaming voice of `track` unless it already has one.
    fn start_music_voice(&mut self, track: &str) {
        if !self.is_available() || self.music_voices.contains(track) {
            return;
        }
        let Some(bytes) = self.music_sources.get(track).cloned() else {
            return;
        };
        let data = match open_stream(&bytes) {
            Ok(data) => data,
            Err(e) => {
                log::warn!("music track `{track}` could not be opened: {e}");
                return;
            }
        };
        if let Some(handle) = self.start_music_stream(data) {
            self.music_voices.insert(track, KiraMusicVoice { handle });
        }
    }
}

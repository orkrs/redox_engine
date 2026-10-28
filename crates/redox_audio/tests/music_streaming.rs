//! Streaming music through kira (plan item B8, last part).
//!
//! API on `AudioContext` (implemented in `music_play.rs`):
//!   * `register_music(name, bytes) -> Result<(), MusicError>`: the bytes of an Ogg / MP3 / FLAC / WAV
//!     file. They are probed by decoding the header (the music is streamed from memory when played), so
//!     garbage is rejected up front. Registering a name again replaces it.
//!   * `register_music_file(name, path) -> Result<(), MusicError>`: the same file kinds, registered by path.
//!     The header is checked now; the file is read from disk when the track plays.
//!   * `has_music(name) -> bool`.
//!   * `play_music(track)` as before, and when an audio device exists and the track is registered, the
//!     track starts streaming in a loop on the Music bus (silent at first; `update_music` raises it).
//!   * `update_music(dt)` advances the crossfade and applies the gains to the streaming voices
//!     (`MusicVoices`, with `KiraMusicVoice` as the `MusicVoice` over a streaming handle).
//!   * `playing_music_voices() -> usize`: how many streaming voices are alive.
//!
//! Tests that need a sound card are skipped when there is none.

#![allow(clippy::unwrap_used)]

use redox_audio::AudioContext;

const TONE: &[u8] = include_bytes!("data/tone.ogg");

#[test]
fn a_valid_file_is_registered_and_garbage_is_rejected() {
    let mut ctx = AudioContext::new();
    assert!(!ctx.has_music("calm"));
    ctx.register_music("calm", TONE.to_vec()).unwrap();
    assert!(ctx.has_music("calm"));
    assert!(
        ctx.register_music("bad", b"not audio at all".to_vec())
            .is_err()
    );
    assert!(!ctx.has_music("bad"));
    assert!(ctx.register_music("empty", Vec::new()).is_err());
    ctx.register_music("calm", TONE.to_vec()).unwrap();
    assert!(ctx.has_music("calm"), "registering again replaces");
}

#[test]
fn the_crossfade_logic_works_with_registered_music_with_or_without_a_device() {
    let mut ctx = AudioContext::new();
    ctx.register_music("calm", TONE.to_vec()).unwrap();
    ctx.set_music_crossfade(1.0);
    ctx.play_music("calm");
    let frame = ctx.update_music(1.0);
    assert_eq!(frame.gains.len(), 1);
    assert!((frame.gains[0].1 - 1.0).abs() < 1e-4);
    ctx.stop_music();
    let end = ctx.update_music(1.0);
    assert_eq!(end.finished, vec!["calm".to_string()]);
    assert_eq!(
        ctx.playing_music_voices(),
        0,
        "a finished track's voice is gone"
    );
}

#[test]
fn an_unregistered_track_plays_no_voice() {
    let mut ctx = AudioContext::new();
    ctx.play_music("ghost");
    ctx.update_music(0.1);
    assert_eq!(ctx.playing_music_voices(), 0);
}

#[test]
fn with_a_device_a_registered_track_streams_and_a_second_one_crossfades() {
    let mut ctx = AudioContext::new();
    if !ctx.is_available() {
        return;
    }
    ctx.register_music("calm", TONE.to_vec()).unwrap();
    ctx.register_music("battle", TONE.to_vec()).unwrap();
    ctx.set_music_crossfade(0.5);
    ctx.play_music("calm");
    ctx.update_music(0.4);
    assert_eq!(ctx.playing_music_voices(), 1);
    ctx.play_music("battle");
    ctx.update_music(0.1);
    assert_eq!(
        ctx.playing_music_voices(),
        2,
        "both are audible during the fade"
    );
    ctx.update_music(1.0);
    assert_eq!(ctx.playing_music_voices(), 1, "calm faded out and stopped");
}

fn temp_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("redox_music_{}_{tag}", std::process::id()))
}

fn tone_path() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/tone.ogg")
}

#[test]
fn a_file_track_registers_and_is_known() {
    let mut ctx = AudioContext::new();
    ctx.register_music_file("tone", tone_path()).unwrap();
    assert!(ctx.has_music("tone"));
}

#[test]
fn a_missing_file_is_rejected_with_an_error() {
    let mut ctx = AudioContext::new();
    assert!(ctx.register_music_file("x", "no/such.ogg").is_err());
    assert!(!ctx.has_music("x"));
}

#[test]
fn a_text_file_is_rejected_as_not_audio() {
    let path = temp_path("text.ogg");
    std::fs::write(&path, b"hello").unwrap();
    let mut ctx = AudioContext::new();
    let result = ctx.register_music_file("text", &path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err());
    assert!(!ctx.has_music("text"));
}

#[test]
fn a_file_track_starts_a_streaming_voice_when_a_device_exists() {
    let mut ctx = AudioContext::new();
    if !ctx.is_available() {
        return;
    }
    ctx.register_music_file("tone", tone_path()).unwrap();
    ctx.play_music("tone");
    ctx.update_music(0.0);
    assert_eq!(ctx.playing_music_voices(), 1);
}

#[test]
fn a_file_removed_after_registration_does_not_panic_on_play() {
    let path = temp_path("removed.ogg");
    std::fs::copy(tone_path(), &path).unwrap();
    let mut ctx = AudioContext::new();
    ctx.register_music_file("gone", &path).unwrap();
    std::fs::remove_file(&path).unwrap();
    ctx.play_music("gone");
    ctx.update_music(0.0);
    assert_eq!(ctx.current_music(), Some("gone"));
}

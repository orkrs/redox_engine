//! Specification (tests first) for wiring `MusicDirector` into `AudioContext` (plan item B8).
//!
//! The director (in `music.rs`) is pure logic. The context should own one and expose it. API to add to
//! `AudioContext` (a new file `crates/redox_audio/src/music_play.rs` with an `impl AudioContext` block is
//! the cleanest place; add `mod music_play;` to `lib.rs`, nothing to re-export):
//!
//! ```text
//! impl AudioContext {
//!     /// Crossfade time for music changes; default 2.0 s. Replaces the director's setting, keeping the
//!     /// current track. Negative or NaN means an instant switch.
//!     pub fn set_music_crossfade(&mut self, seconds: f32);
//!     /// Starts `track` fading in and fades the others out. Same track again does nothing.
//!     pub fn play_music(&mut self, track: &str);
//!     /// Fades all music out.
//!     pub fn stop_music(&mut self);
//!     /// The selected track, if any.
//!     pub fn current_music(&self) -> Option<&str>;
//!     /// Advances the fades by `dt` seconds and returns the frame (gains of audible tracks, tracks that
//!     /// finished). Works without an audio device. With a device and a registered sound named like the
//!     /// track (`register_sfx_data`), the implementation may also start it looped and set its volume
//!     /// to the gain; that part is optional and not tested.
//!     pub fn update_music(&mut self, dt: f32) -> MusicFrame;
//! }
//! ```
//!
//! Hint: keep a `MusicDirector` field in `AudioContext`. `MusicDirector::new(seconds)` fixes the
//! crossfade at construction, so `set_music_crossfade` rebuilds it and replays the current track.

#![allow(clippy::unwrap_used)]

use redox_audio::AudioContext;

fn gain_of(frame: &redox_audio::MusicFrame, name: &str) -> f32 {
    frame
        .gains
        .iter()
        .find(|(n, _)| n == name)
        .map_or(0.0, |(_, g)| *g)
}

#[test]
fn a_new_context_plays_no_music() {
    let mut ctx = AudioContext::new();
    assert_eq!(ctx.current_music(), None);
    assert!(ctx.update_music(0.1).gains.is_empty());
}

#[test]
fn a_track_fades_in_over_the_crossfade_time() {
    let mut ctx = AudioContext::new();
    ctx.set_music_crossfade(1.0);
    ctx.play_music("calm");
    assert_eq!(ctx.current_music(), Some("calm"));
    let half = ctx.update_music(0.5);
    let full = ctx.update_music(0.5);
    assert!(
        (gain_of(&half, "calm") - 0.5_f32.sqrt()).abs() < 1e-3,
        "equal power at half"
    );
    assert!((gain_of(&full, "calm") - 1.0).abs() < 1e-4);
}

#[test]
fn switching_tracks_crossfades_with_equal_power() {
    let mut ctx = AudioContext::new();
    ctx.set_music_crossfade(1.0);
    ctx.play_music("calm");
    ctx.update_music(1.0);
    ctx.play_music("battle");
    let frame = ctx.update_music(0.5);
    let (a, b) = (gain_of(&frame, "calm"), gain_of(&frame, "battle"));
    assert!(
        (a * a + b * b - 1.0).abs() < 1e-3,
        "power stays constant: {a} {b}"
    );
    let end = ctx.update_music(0.5);
    assert_eq!(end.finished, vec!["calm".to_string()]);
    assert_eq!(ctx.current_music(), Some("battle"));
}

#[test]
fn stopping_fades_everything_out() {
    let mut ctx = AudioContext::new();
    ctx.set_music_crossfade(0.5);
    ctx.play_music("calm");
    ctx.update_music(0.5);
    ctx.stop_music();
    assert_eq!(ctx.current_music(), None);
    let frame = ctx.update_music(0.5);
    assert!(frame.gains.is_empty());
    assert_eq!(frame.finished, vec!["calm".to_string()]);
}

#[test]
fn a_negative_crossfade_switches_instantly() {
    let mut ctx = AudioContext::new();
    ctx.set_music_crossfade(-1.0);
    ctx.play_music("a");
    assert!((gain_of(&ctx.update_music(0.0), "a") - 1.0).abs() < 1e-4);
}

#[test]
fn changing_the_crossfade_keeps_the_current_track() {
    let mut ctx = AudioContext::new();
    ctx.play_music("calm");
    ctx.set_music_crossfade(0.25);
    assert_eq!(ctx.current_music(), Some("calm"));
    let frame = ctx.update_music(0.25);
    assert!((gain_of(&frame, "calm") - 1.0).abs() < 1e-4);
}

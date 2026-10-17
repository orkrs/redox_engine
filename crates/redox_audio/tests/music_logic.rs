//! Specification (tests first) for the music crossfade logic and the priority voice budget
//! (plan item B8, remaining part). Both are pure state machines: no audio device, no sound data.
//!
//! API to implement in `redox_audio` (new modules `music.rs` and `voices.rs`, re-exported from the
//! crate root):
//!
//! ```text
//! pub struct MusicFrame { pub gains: Vec<(String, f32)>, pub finished: Vec<String> }
//! pub struct MusicDirector;
//! impl MusicDirector {
//!     /// `crossfade_seconds <= 0` switches instantly.
//!     pub fn new(crossfade_seconds: f32) -> Self;
//!     /// Starts `track` fading in while every other track fades out. Playing the track that is
//!     /// already current is a no-op. A track that is still fading out turns around from its
//!     /// current level (no jump).
//!     pub fn play(&mut self, track: &str);
//!     /// Fades everything out; `current()` is `None` immediately.
//!     pub fn stop(&mut self);
//!     pub fn current(&self) -> Option<&str>;
//!     /// True when no track is audible any more.
//!     pub fn is_idle(&self) -> bool;
//!     /// Advances all fades by `dt` seconds (negative or NaN `dt` changes nothing). `gains` lists every
//!     /// still audible track (in the order they were started); `finished` lists the tracks that
//!     /// reached silence in this update and were removed (reported exactly once).
//!     pub fn update(&mut self, dt: f32) -> MusicFrame;
//! }
//!
//! pub enum VoiceDecision { Play, Steal(u64), Reject }
//! pub struct VoiceBudget;
//! impl VoiceBudget {
//!     pub fn new(max_voices: usize) -> Self;
//!     /// A voice with `id`, `priority` (higher wins) starts at `now` and occupies a slot for `duration`.
//!     /// Free slot (after expiring finished voices) -> `Play`. Full: if some active voice has a STRICTLY
//!     /// lower priority, the lowest-priority one (the oldest among equals) is stolen -> `Steal(its id)`
//!     /// and the new voice takes its place; otherwise `Reject` and nothing changes.
//!     pub fn request(&mut self, id: u64, priority: u8, now: f32, duration: f32) -> VoiceDecision;
//!     pub fn active(&self, now: f32) -> usize;
//! }
//! ```
//!
//! Fade law: each track has a level in [0, 1] that moves linearly (rate 1 / crossfade seconds) towards
//! 1 for the current track and 0 for all others; the gain reported is `sin(level * PI / 2)`
//! (equal power: a symmetrical crossfade keeps gain_a^2 + gain_b^2 = 1).

#![allow(clippy::unwrap_used)]

use std::f32::consts::FRAC_PI_2;

use redox_audio::{MusicDirector, VoiceBudget, VoiceDecision};

fn gain_of(frame: &redox_audio::MusicFrame, track: &str) -> Option<f32> {
    frame
        .gains
        .iter()
        .find(|(t, _)| t == track)
        .map(|(_, g)| *g)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn the_first_track_fades_in_from_silence() {
    let mut music = MusicDirector::new(1.0);
    assert!(music.is_idle());
    music.play("explore");
    assert_eq!(music.current(), Some("explore"));
    let half = music.update(0.5);
    assert!(close(
        gain_of(&half, "explore").unwrap(),
        (0.5f32 * FRAC_PI_2).sin()
    ));
    let full = music.update(0.5);
    assert!(close(gain_of(&full, "explore").unwrap(), 1.0));
    assert!(full.finished.is_empty());
    assert!(!music.is_idle());
}

#[test]
fn a_crossfade_is_equal_power_and_retires_the_old_track() {
    let mut music = MusicDirector::new(1.0);
    music.play("calm");
    music.update(1.0);
    music.play("battle");
    assert_eq!(music.current(), Some("battle"));

    let mid = music.update(0.5);
    let (a, b) = (
        gain_of(&mid, "calm").unwrap(),
        gain_of(&mid, "battle").unwrap(),
    );
    assert!(close(a, b), "symmetrical at the midpoint: {a} {b}");
    assert!(close(a * a + b * b, 1.0), "equal power: {}", a * a + b * b);
    assert_eq!(
        mid.gains
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>(),
        vec!["calm", "battle"],
        "in start order"
    );

    let end = music.update(0.5);
    assert_eq!(end.finished, vec!["calm"]);
    assert_eq!(gain_of(&end, "calm"), None);
    assert!(close(gain_of(&end, "battle").unwrap(), 1.0));
    assert!(music.update(0.1).finished.is_empty(), "reported once");
}

#[test]
fn playing_the_current_track_again_does_nothing() {
    let mut music = MusicDirector::new(1.0);
    music.play("calm");
    music.update(0.4);
    music.play("calm");
    let frame = music.update(0.1);
    // Still on the same fade: level 0.5, not restarted from 0.
    assert!(close(
        gain_of(&frame, "calm").unwrap(),
        (0.5f32 * FRAC_PI_2).sin()
    ));
}

#[test]
fn turning_a_fading_track_around_does_not_jump() {
    let mut music = MusicDirector::new(1.0);
    music.play("a");
    music.update(1.0);
    music.play("b");
    let fading = music.update(0.6); // a is at level 0.4
    let before = gain_of(&fading, "a").unwrap();
    assert!(close(before, (0.4f32 * FRAC_PI_2).sin()));
    music.play("a"); // back to a
    let after = gain_of(&music.update(0.1), "a").unwrap(); // level 0.5
    assert!(after > before, "a rises again: {before} -> {after}");
    assert!(
        close(after, (0.5f32 * FRAC_PI_2).sin()),
        "continues from 0.4 upwards"
    );
}

#[test]
fn stop_fades_everything_out_and_goes_idle() {
    let mut music = MusicDirector::new(2.0);
    music.play("calm");
    music.update(2.0);
    music.stop();
    assert_eq!(music.current(), None);
    assert!(!music.is_idle(), "still fading");
    let mid = music.update(1.0);
    assert!(close(
        gain_of(&mid, "calm").unwrap(),
        (0.5f32 * FRAC_PI_2).sin()
    ));
    let end = music.update(1.0);
    assert_eq!(end.finished, vec!["calm"]);
    assert!(music.is_idle());
}

#[test]
fn a_zero_crossfade_switches_instantly() {
    let mut music = MusicDirector::new(0.0);
    music.play("a");
    let frame = music.update(0.0);
    assert!(close(gain_of(&frame, "a").unwrap(), 1.0));
    music.play("b");
    let frame = music.update(0.0);
    assert_eq!(frame.finished, vec!["a"]);
    assert!(close(gain_of(&frame, "b").unwrap(), 1.0));
}

#[test]
fn bad_time_steps_change_nothing() {
    let mut music = MusicDirector::new(1.0);
    music.play("a");
    music.update(0.5);
    let reference = music.update(0.0);
    for dt in [-1.0, f32::NAN] {
        let frame = music.update(dt);
        assert_eq!(frame.gains, reference.gains);
        assert!(frame.finished.is_empty());
    }
}

#[test]
fn a_free_slot_plays() {
    let mut budget = VoiceBudget::new(2);
    assert_eq!(budget.request(1, 5, 0.0, 1.0), VoiceDecision::Play);
    assert_eq!(budget.request(2, 5, 0.1, 1.0), VoiceDecision::Play);
    assert_eq!(budget.active(0.2), 2);
}

#[test]
fn a_full_budget_rejects_equal_or_lower_priority() {
    let mut budget = VoiceBudget::new(2);
    budget.request(1, 5, 0.0, 10.0);
    budget.request(2, 7, 0.0, 10.0);
    assert_eq!(budget.request(3, 5, 1.0, 1.0), VoiceDecision::Reject);
    assert_eq!(budget.request(4, 1, 1.0, 1.0), VoiceDecision::Reject);
    assert_eq!(budget.active(1.0), 2, "rejections change nothing");
}

#[test]
fn a_higher_priority_steals_the_lowest_and_oldest() {
    let mut budget = VoiceBudget::new(3);
    budget.request(1, 4, 0.0, 10.0);
    budget.request(2, 4, 1.0, 10.0);
    budget.request(3, 6, 2.0, 10.0);
    // Priority 4 is the lowest; between the two the older one (id 1) goes.
    assert_eq!(budget.request(9, 8, 3.0, 10.0), VoiceDecision::Steal(1));
    assert_eq!(budget.active(3.0), 3);
    assert_eq!(budget.request(10, 8, 3.5, 10.0), VoiceDecision::Steal(2));
    assert_eq!(budget.request(11, 8, 4.0, 10.0), VoiceDecision::Steal(3));
    assert_eq!(
        budget.request(12, 8, 4.5, 10.0),
        VoiceDecision::Reject,
        "equal priority does not steal"
    );
}

#[test]
fn finished_voices_free_their_slots() {
    let mut budget = VoiceBudget::new(1);
    assert_eq!(budget.request(1, 5, 0.0, 1.0), VoiceDecision::Play);
    assert_eq!(budget.request(2, 5, 0.5, 1.0), VoiceDecision::Reject);
    assert_eq!(budget.active(1.0), 0, "a voice ends at start + duration");
    assert_eq!(budget.request(3, 5, 1.0, 1.0), VoiceDecision::Play);
}

#[test]
fn a_zero_budget_plays_nothing() {
    let mut budget = VoiceBudget::new(0);
    assert_eq!(budget.request(1, 255, 0.0, 1.0), VoiceDecision::Reject);
}

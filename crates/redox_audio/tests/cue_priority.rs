//! Specification (tests first) for cue priorities in `AudioContext::play_cue` (plan item B8, remaining
//! part). `SoundCue.priority` is stored today but never used. `VoiceBudget` (in `voices.rs`, already
//! implemented) decides who may play.
//!
//! API to add to `AudioContext` (in `context.rs` or `cues.rs`; the context owns a `VoiceBudget`):
//!
//! ```text
//! impl AudioContext {
//!     /// How many cue voices may sound at once. Default 16. Changing it starts a fresh budget.
//!     pub fn set_cue_voice_limit(&mut self, max_voices: usize);
//!     /// How long, in seconds, a started cue is assumed to occupy its voice (the sounds' real lengths are
//!     /// not known to the context). Default 1.0. Must be finite and > 0, otherwise ignored.
//!     pub fn set_cue_voice_seconds(&mut self, seconds: f32);
//! }
//! ```
//!
//! `play_cue(library, selector, name, now)` keeps its signature and its behaviour without a device. New
//! rule: after the selector chose a variant, the cue asks the voice budget with its `priority` and `now`.
//! `Play` -> as before. `Steal(victim)` -> as before (the oldest lower-priority voice is dropped; stopping
//! its audio handle is allowed but optional, there is no handle without a device). `Reject` ->
//! `play_cue` returns `None` and plays nothing. Voice ids are the context's own counter. The cooldown of
//! the rejected cue has already been consumed by the selection; that is accepted.
//!
//! Do not change `SoundCue`'s fields (`tests/sound_cues.rs` builds it with a struct literal).

#![allow(clippy::unwrap_used)]

use redox_audio::{AudioContext, CueLibrary, CueSelector};

const LIBRARY: &str = r#"[
    (name: "step", variants: ["s1", "s2"], volume: (1.0, 1.0), pitch: (1.0, 1.0),
     cooldown: 0.0, bus: Sfx, priority: 2),
    (name: "hit", variants: ["h1", "h2"], volume: (1.0, 1.0), pitch: (1.0, 1.0),
     cooldown: 0.0, bus: Sfx, priority: 6),
    (name: "boom", variants: ["b1", "b2"], volume: (1.0, 1.0), pitch: (1.0, 1.0),
     cooldown: 0.0, bus: Sfx, priority: 9),
]"#;

fn setup(limit: usize) -> (AudioContext, CueLibrary, CueSelector) {
    let mut ctx = AudioContext::new();
    ctx.set_cue_voice_limit(limit);
    ctx.set_cue_voice_seconds(1.0);
    (ctx, CueLibrary::parse(LIBRARY).unwrap(), CueSelector::new())
}

#[test]
fn cues_play_while_voices_are_free() {
    let (mut ctx, lib, mut sel) = setup(2);
    assert!(ctx.play_cue(&lib, &mut sel, "step", 0.0).is_some());
    assert!(ctx.play_cue(&lib, &mut sel, "step", 0.1).is_some());
}

#[test]
fn an_equal_or_lower_priority_cue_is_rejected_when_full() {
    let (mut ctx, lib, mut sel) = setup(2);
    ctx.play_cue(&lib, &mut sel, "hit", 0.0).unwrap();
    ctx.play_cue(&lib, &mut sel, "hit", 0.1).unwrap();
    assert!(
        ctx.play_cue(&lib, &mut sel, "hit", 0.2).is_none(),
        "equal priority"
    );
    assert!(
        ctx.play_cue(&lib, &mut sel, "step", 0.3).is_none(),
        "lower priority"
    );
}

#[test]
fn a_higher_priority_cue_steals_the_lowest() {
    let (mut ctx, lib, mut sel) = setup(2);
    ctx.play_cue(&lib, &mut sel, "step", 0.0).unwrap();
    ctx.play_cue(&lib, &mut sel, "hit", 0.1).unwrap();
    assert!(
        ctx.play_cue(&lib, &mut sel, "boom", 0.2).is_some(),
        "steals the step"
    );
    // Now hit(6) and boom(9) are active: a step cannot get in, another boom cannot steal boom (equal),
    // but it can steal the hit.
    assert!(ctx.play_cue(&lib, &mut sel, "step", 0.3).is_none());
    assert!(
        ctx.play_cue(&lib, &mut sel, "boom", 0.4).is_some(),
        "steals the hit"
    );
    assert!(
        ctx.play_cue(&lib, &mut sel, "boom", 0.5).is_none(),
        "two booms, none lower"
    );
}

#[test]
fn voices_are_free_again_after_their_duration() {
    let (mut ctx, lib, mut sel) = setup(1);
    ctx.set_cue_voice_seconds(0.5);
    assert!(ctx.play_cue(&lib, &mut sel, "hit", 0.0).is_some());
    assert!(ctx.play_cue(&lib, &mut sel, "hit", 0.4).is_none());
    assert!(ctx.play_cue(&lib, &mut sel, "hit", 0.5).is_some());
}

#[test]
fn a_zero_limit_plays_nothing_and_a_bad_duration_is_ignored() {
    let (mut ctx, lib, mut sel) = setup(0);
    assert!(ctx.play_cue(&lib, &mut sel, "boom", 0.0).is_none());

    let (mut ctx, lib, mut sel) = setup(1);
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        ctx.set_cue_voice_seconds(bad);
    }
    // The duration stayed 1.0.
    assert!(ctx.play_cue(&lib, &mut sel, "hit", 0.0).is_some());
    assert!(ctx.play_cue(&lib, &mut sel, "hit", 0.9).is_none());
    assert!(ctx.play_cue(&lib, &mut sel, "hit", 1.0).is_some());
}

#[test]
fn unknown_cues_still_return_none_and_take_no_voice() {
    let (mut ctx, lib, mut sel) = setup(1);
    assert!(ctx.play_cue(&lib, &mut sel, "nope", 0.0).is_none());
    assert!(ctx.play_cue(&lib, &mut sel, "step", 0.0).is_some());
}

#[test]
fn the_default_limit_allows_many_cues() {
    let mut ctx = AudioContext::new();
    let lib = CueLibrary::parse(LIBRARY).unwrap();
    let mut sel = CueSelector::new();
    let played = (0..16)
        .filter(|i| {
            ctx.play_cue(&lib, &mut sel, "step", *i as f32 * 0.01)
                .is_some()
        })
        .count();
    assert_eq!(played, 16);
    assert!(
        ctx.play_cue(&lib, &mut sel, "step", 0.2).is_none(),
        "the 17th"
    );
}

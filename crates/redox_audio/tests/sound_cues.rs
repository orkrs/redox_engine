//! Specification (tests first) for data-driven sound cues (plan item B8, first part).
//!
//! API to implement in `redox_audio` (new module `cues.rs`, re-exported from the crate root):
//!
//! ```text
//! #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
//! pub struct SoundCue {
//!     pub name: String,
//!     pub variants: Vec<String>,        // sound asset names, e.g. "step1"
//!     pub volume: (f32, f32),           // linear gain range, picked uniformly
//!     pub pitch: (f32, f32),            // playback-rate range, picked uniformly
//!     pub cooldown: f32,                // seconds that must pass before the cue can play again
//!     pub bus: MixerBusId,              // must derive Serialize/Deserialize (add it, plus serde to Cargo.toml)
//!     pub priority: u8,
//! }
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! pub struct CuePlay { pub variant: usize, pub volume: f32, pub pitch: f32 }
//! pub trait CueRng { fn next_f32(&mut self) -> f32 }        // uniform in [0, 1)
//! pub struct SplitMix64(pub u64);  impl CueRng for SplitMix64 // the SplitMix64 generator
//! pub struct CueSelector;          impl Default; impl CueSelector { pub fn new() -> Self;
//!     pub fn select(&mut self, cue: &SoundCue, now: f32, rng: &mut dyn CueRng) -> Option<CuePlay>; }
//! pub struct CueLibrary;           impl CueLibrary { pub fn parse(ron_text: &str) -> Result<Self, CueError>;
//!     pub fn get(&self, name: &str) -> Option<&SoundCue>; pub fn len(&self) -> usize; pub fn is_empty(&self) -> bool; }
//! pub enum CueError { Parse(String), DuplicateName(String), Invalid(String) }   // Debug + Display (thiserror)
//! impl AudioContext { pub fn play_cue(&mut self, library: &CueLibrary, selector: &mut CueSelector,
//!                                     name: &str, now: f32) -> Option<CuePlay>; }
//! ```
//!
//! Rules of `select`: no variants -> `None`; while `now - last_play < cooldown` -> `None` and the
//! rejected call changes nothing; with two or more variants the same variant is never chosen twice in
//! a row (for that cue name); volume and pitch are uniform in their ranges (a degenerate range gives
//! exactly that value); state (last variant, last play time) is kept per cue name inside the selector.
//! `now` going backwards must not panic (treat the cue as ready).
//!
//! `CueLibrary::parse` reads RON: a list of `SoundCue`; empty list is fine; duplicate names ->
//! `DuplicateName`; a range with min > max, a negative cooldown or an empty `name` -> `Invalid`.
//!
//! `AudioContext::play_cue` looks the cue up, selects a variant, and plays that sound asset on the
//! cue's bus through the existing `play_sfx` machinery. Without an audio device or loaded sounds it
//! must not panic (returns whatever `select` chose, or `None` for an unknown cue name).

#![allow(clippy::unwrap_used)]

use redox_audio::{
    AudioContext, CueError, CueLibrary, CuePlay, CueRng, CueSelector, MixerBusId, SoundCue,
    SplitMix64,
};

fn cue(variants: &[&str]) -> SoundCue {
    SoundCue {
        name: "footstep".into(),
        variants: variants.iter().map(|v| v.to_string()).collect(),
        volume: (0.8, 1.0),
        pitch: (0.95, 1.05),
        cooldown: 0.0,
        bus: MixerBusId::Sfx,
        priority: 5,
    }
}

#[test]
fn the_same_variant_is_never_played_twice_in_a_row() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(1);
    let cue = cue(&["a", "b", "c"]);
    let mut previous = None;
    for i in 0..2000 {
        let play = selector.select(&cue, i as f32, &mut rng).unwrap();
        assert_ne!(Some(play.variant), previous, "repeat at selection {i}");
        previous = Some(play.variant);
    }
}

#[test]
fn every_variant_gets_played() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(7);
    let cue = cue(&["a", "b", "c", "d"]);
    let mut seen = [0u32; 4];
    for i in 0..2000 {
        seen[selector.select(&cue, i as f32, &mut rng).unwrap().variant] += 1;
    }
    assert!(seen.iter().all(|&n| n > 300), "roughly even use: {seen:?}");
}

#[test]
fn volume_and_pitch_stay_in_range_and_actually_vary() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(3);
    let cue = cue(&["a", "b"]);
    let plays: Vec<CuePlay> = (0..1000)
        .map(|i| selector.select(&cue, i as f32, &mut rng).unwrap())
        .collect();
    assert!(plays.iter().all(|p| (0.8..=1.0).contains(&p.volume)));
    assert!(plays.iter().all(|p| (0.95..=1.05).contains(&p.pitch)));
    let (min, max) = plays
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.volume), hi.max(p.volume)));
    assert!(max - min > 0.15, "volume spreads over its range: {min}..{max}");
}

#[test]
fn a_degenerate_range_gives_exactly_that_value() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(5);
    let mut fixed = cue(&["a"]);
    fixed.volume = (0.5, 0.5);
    fixed.pitch = (1.0, 1.0);
    let play = selector.select(&fixed, 0.0, &mut rng).unwrap();
    assert_eq!((play.volume, play.pitch), (0.5, 1.0));
}

#[test]
fn the_cooldown_blocks_replays_and_rejections_change_nothing() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(9);
    let mut c = cue(&["a", "b"]);
    c.cooldown = 0.1;
    assert!(selector.select(&c, 0.0, &mut rng).is_some());
    assert!(selector.select(&c, 0.05, &mut rng).is_none(), "inside the cooldown");
    assert!(selector.select(&c, 0.09, &mut rng).is_none(), "still inside; the 0.05 call did not restart it");
    assert!(selector.select(&c, 0.1, &mut rng).is_some(), "exactly at the cooldown");
}

#[test]
fn cooldowns_are_per_cue() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(11);
    let mut step = cue(&["a"]);
    step.cooldown = 1.0;
    let mut jump = cue(&["j"]);
    jump.name = "jump".into();
    jump.cooldown = 1.0;
    assert!(selector.select(&step, 0.0, &mut rng).is_some());
    assert!(selector.select(&jump, 0.0, &mut rng).is_some(), "another cue is not blocked");
    assert!(selector.select(&step, 0.5, &mut rng).is_none());
}

#[test]
fn single_variants_may_repeat_and_empty_cues_play_nothing() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(13);
    let one = cue(&["only"]);
    for i in 0..5 {
        assert_eq!(selector.select(&one, i as f32, &mut rng).unwrap().variant, 0);
    }
    assert!(selector.select(&cue(&[]), 0.0, &mut rng).is_none());
}

#[test]
fn time_going_backwards_does_not_panic() {
    let mut selector = CueSelector::new();
    let mut rng = SplitMix64(17);
    let mut c = cue(&["a", "b"]);
    c.cooldown = 1.0;
    assert!(selector.select(&c, 10.0, &mut rng).is_some());
    assert!(selector.select(&c, 2.0, &mut rng).is_some(), "clock reset: the cue is ready again");
}

#[test]
fn the_generator_is_deterministic_and_uniform_enough() {
    let mut a = SplitMix64(42);
    let mut b = SplitMix64(42);
    let xs: Vec<f32> = (0..1000).map(|_| a.next_f32()).collect();
    let ys: Vec<f32> = (0..1000).map(|_| b.next_f32()).collect();
    assert_eq!(xs, ys);
    assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
    let mean = xs.iter().sum::<f32>() / xs.len() as f32;
    assert!((mean - 0.5).abs() < 0.05, "mean {mean}");
}

const LIBRARY: &str = r#"[
    (name: "footstep", variants: ["step1", "step2", "step3"], volume: (0.8, 1.0),
     pitch: (0.95, 1.05), cooldown: 0.1, bus: Sfx, priority: 5),
    (name: "coin", variants: ["coin"], volume: (1.0, 1.0),
     pitch: (1.0, 1.2), cooldown: 0.0, bus: Ui, priority: 8),
]"#;

#[test]
fn libraries_parse_from_ron() {
    let lib = CueLibrary::parse(LIBRARY).unwrap();
    assert_eq!(lib.len(), 2);
    let step = lib.get("footstep").unwrap();
    assert_eq!(step.variants, vec!["step1", "step2", "step3"]);
    assert_eq!(step.bus, MixerBusId::Sfx);
    assert_eq!(lib.get("coin").unwrap().bus, MixerBusId::Ui);
    assert!(lib.get("missing").is_none());
    assert!(CueLibrary::parse("[]").unwrap().is_empty());
}

#[test]
fn bad_libraries_are_rejected_with_a_specific_error() {
    assert!(matches!(CueLibrary::parse("not ron at all"), Err(CueError::Parse(_))));
    let dup = r#"[(name: "a", variants: ["x"], volume: (1.0, 1.0), pitch: (1.0, 1.0), cooldown: 0.0, bus: Sfx, priority: 1),
                  (name: "a", variants: ["y"], volume: (1.0, 1.0), pitch: (1.0, 1.0), cooldown: 0.0, bus: Sfx, priority: 1)]"#;
    assert!(matches!(CueLibrary::parse(dup), Err(CueError::DuplicateName(n)) if n == "a"));
    let backwards = r#"[(name: "a", variants: ["x"], volume: (1.0, 0.5), pitch: (1.0, 1.0), cooldown: 0.0, bus: Sfx, priority: 1)]"#;
    assert!(matches!(CueLibrary::parse(backwards), Err(CueError::Invalid(_))));
    let negative = r#"[(name: "a", variants: ["x"], volume: (1.0, 1.0), pitch: (1.0, 1.0), cooldown: -1.0, bus: Sfx, priority: 1)]"#;
    assert!(matches!(CueLibrary::parse(negative), Err(CueError::Invalid(_))));
    let unnamed = r#"[(name: "", variants: ["x"], volume: (1.0, 1.0), pitch: (1.0, 1.0), cooldown: 0.0, bus: Sfx, priority: 1)]"#;
    assert!(matches!(CueLibrary::parse(unnamed), Err(CueError::Invalid(_))));
}

#[test]
fn playing_cues_without_a_device_or_sounds_never_panics() {
    let lib = CueLibrary::parse(LIBRARY).unwrap();
    let mut ctx = AudioContext::new();
    let mut selector = CueSelector::new();
    assert!(ctx.play_cue(&lib, &mut selector, "nope", 0.0).is_none(), "unknown cue");
    let play = ctx.play_cue(&lib, &mut selector, "footstep", 0.0);
    assert!(play.is_none() || play.unwrap().variant < 3);
}

//! Specification (tests first) for applying music frames to playing voices (plan item B8, music).
//!
//! `MusicDirector::update` (and `AudioContext::update_music`) produce a `MusicFrame`: gains of audible
//! tracks and tracks that finished. Something has to turn that into volumes on real players. That
//! something is device independent, so it is written against a trait and tested with fakes; the kira
//! implementation of the trait comes later. New file `crates/redox_audio/src/music_apply.rs`
//! (`mod music_apply;` in `lib.rs`, re-export `MusicVoice` and `MusicVoices` from the crate root):
//!
//! ```text
//! /// One playing music track.
//! pub trait MusicVoice {
//!     /// Sets the linear volume, already clamped to [0, 1].
//!     fn set_volume(&mut self, gain: f32);
//!     /// Stops the track for good.
//!     fn stop(&mut self);
//! }
//! /// The playing tracks by name.
//! pub struct MusicVoices<V: MusicVoice> { /* HashMap<String, V> */ }
//! impl<V: MusicVoice> MusicVoices<V> {
//!     pub fn new() -> Self;                         // also `Default`
//!     /// Adds a voice under `name`. An existing voice of that name is stopped and replaced.
//!     pub fn insert(&mut self, name: impl Into<String>, voice: V);
//!     pub fn contains(&self, name: &str) -> bool;
//!     pub fn len(&self) -> usize;
//!     pub fn is_empty(&self) -> bool;
//!     /// For every `(name, gain)` of the frame that has a voice: `set_volume(gain.clamp(0, 1))`
//!     /// (NaN counts as 0). For every name in `frame.finished`: `stop()` the voice and remove it.
//!     /// Names without a voice are ignored. Voices not mentioned in the frame are left alone.
//!     pub fn apply(&mut self, frame: &MusicFrame);
//! }
//! ```

#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::rc::Rc;

use redox_audio::{MusicDirector, MusicFrame, MusicVoice, MusicVoices};

#[derive(Default)]
struct Log {
    volumes: Vec<f32>,
    stopped: u32,
}

#[derive(Clone, Default)]
struct Fake(Rc<RefCell<Log>>);

impl MusicVoice for Fake {
    fn set_volume(&mut self, gain: f32) {
        self.0.borrow_mut().volumes.push(gain);
    }
    fn stop(&mut self) {
        self.0.borrow_mut().stopped += 1;
    }
}

fn frame(gains: &[(&str, f32)], finished: &[&str]) -> MusicFrame {
    MusicFrame {
        gains: gains.iter().map(|(n, g)| (n.to_string(), *g)).collect(),
        finished: finished.iter().map(|n| n.to_string()).collect(),
    }
}

#[test]
fn gains_reach_the_named_voices() {
    let (a, b) = (Fake::default(), Fake::default());
    let mut voices = MusicVoices::new();
    voices.insert("calm", a.clone());
    voices.insert("battle", b.clone());
    voices.apply(&frame(&[("calm", 0.25), ("battle", 0.75)], &[]));
    assert_eq!(a.0.borrow().volumes, vec![0.25]);
    assert_eq!(b.0.borrow().volumes, vec![0.75]);
    assert_eq!(voices.len(), 2);
}

#[test]
fn unknown_names_are_ignored_and_unmentioned_voices_are_left_alone() {
    let a = Fake::default();
    let mut voices = MusicVoices::new();
    voices.insert("calm", a.clone());
    voices.apply(&frame(&[("ghost", 1.0)], &["ghost"]));
    assert!(a.0.borrow().volumes.is_empty());
    assert_eq!(a.0.borrow().stopped, 0);
    assert!(voices.contains("calm"));
}

#[test]
fn gains_are_clamped_and_nan_is_silence() {
    let a = Fake::default();
    let mut voices = MusicVoices::new();
    voices.insert("calm", a.clone());
    voices.apply(&frame(&[("calm", 1.7)], &[]));
    voices.apply(&frame(&[("calm", -0.2)], &[]));
    voices.apply(&frame(&[("calm", f32::NAN)], &[]));
    assert_eq!(a.0.borrow().volumes, vec![1.0, 0.0, 0.0]);
}

#[test]
fn finished_tracks_are_stopped_and_removed_once() {
    let a = Fake::default();
    let mut voices = MusicVoices::new();
    voices.insert("calm", a.clone());
    voices.apply(&frame(&[], &["calm"]));
    assert_eq!(a.0.borrow().stopped, 1);
    assert!(!voices.contains("calm"));
    assert!(voices.is_empty());
    voices.apply(&frame(&[], &["calm"]));
    assert_eq!(a.0.borrow().stopped, 1, "not stopped twice");
}

#[test]
fn replacing_a_voice_stops_the_old_one() {
    let (old, new) = (Fake::default(), Fake::default());
    let mut voices = MusicVoices::new();
    voices.insert("calm", old.clone());
    voices.insert("calm", new.clone());
    assert_eq!(old.0.borrow().stopped, 1);
    assert_eq!(new.0.borrow().stopped, 0);
    assert_eq!(voices.len(), 1);
}

#[test]
fn a_whole_crossfade_drives_the_voices_and_ends_with_one() {
    let (calm, battle) = (Fake::default(), Fake::default());
    let mut voices = MusicVoices::new();
    voices.insert("calm", calm.clone());
    voices.insert("battle", battle.clone());
    let mut director = MusicDirector::new(1.0);
    director.play("calm");
    voices.apply(&director.update(1.0));
    director.play("battle");
    for _ in 0..4 {
        voices.apply(&director.update(0.25));
    }
    assert_eq!(calm.0.borrow().stopped, 1, "calm faded out and was stopped");
    assert_eq!(battle.0.borrow().stopped, 0);
    assert!((battle.0.borrow().volumes.last().copied().unwrap() - 1.0).abs() < 1e-4);
    assert_eq!(voices.len(), 1);
    assert!(voices.contains("battle"));
}

//! Specification (tests first) for releasing and shrinking a `VoiceBudget` (plan item B8: stop a stolen
//! or finished voice). Today `request` reports `Steal(id)`, but the caller cannot free a slot early (a
//! sound that was stopped) nor learn which voices a smaller limit evicts.
//!
//! API to add to `VoiceBudget` in `voices.rs`:
//!
//! ```text
//! /// Frees the slot of voice `id` before its time is up. False when no such voice is held.
//! pub fn release(&mut self, id: u64) -> bool;
//! /// Ids of the voices still playing at `now`, oldest first (by start time; ties by insertion order).
//! pub fn ids(&self, now: f32) -> Vec<u64>;
//! /// Lowers (or raises) the limit. Returns the ids evicted to fit the new limit, lowest priority first
//! /// and the oldest first among equals. Voices that already ended by `now` are dropped silently and are
//! /// not reported.
//! pub fn set_max_voices(&mut self, max_voices: usize, now: f32) -> Vec<u64>;
//! ```
//!
//! Wiring into `AudioContext` (which has no device-free test): keep the playing sound handle of each cue
//! voice by id in a `HashMap<u64, StaticSoundHandle>`; on `Steal(victim)` stop and drop the victim's
//! handle; `set_cue_voice_limit` stops the evicted handles. Handles of ended voices are dropped lazily.

#![allow(clippy::unwrap_used)]

use redox_audio::{VoiceBudget, VoiceDecision};

#[test]
fn releasing_a_voice_frees_its_slot() {
    let mut budget = VoiceBudget::new(1);
    budget.request(1, 5, 0.0, 10.0);
    assert_eq!(budget.request(2, 5, 0.1, 1.0), VoiceDecision::Reject);
    assert!(budget.release(1));
    assert_eq!(budget.request(2, 5, 0.2, 1.0), VoiceDecision::Play);
}

#[test]
fn releasing_an_unknown_voice_reports_false() {
    let mut budget = VoiceBudget::new(2);
    assert!(!budget.release(99));
    budget.request(1, 1, 0.0, 1.0);
    assert!(budget.release(1));
    assert!(!budget.release(1), "only once");
}

#[test]
fn ids_lists_the_playing_voices_oldest_first() {
    let mut budget = VoiceBudget::new(4);
    budget.request(10, 1, 2.0, 5.0);
    budget.request(11, 1, 1.0, 5.0);
    budget.request(12, 1, 3.0, 0.5);
    assert_eq!(budget.ids(3.1), vec![11, 10, 12]);
}

#[test]
fn ids_skips_voices_that_ended() {
    let mut budget = VoiceBudget::new(4);
    budget.request(1, 1, 0.0, 1.0);
    budget.request(2, 1, 0.0, 10.0);
    assert_eq!(budget.ids(5.0), vec![2]);
    assert!(budget.ids(20.0).is_empty());
}

#[test]
fn shrinking_evicts_the_weakest_and_reports_them() {
    let mut budget = VoiceBudget::new(4);
    budget.request(1, 5, 0.0, 10.0);
    budget.request(2, 1, 1.0, 10.0);
    budget.request(3, 1, 2.0, 10.0);
    budget.request(4, 9, 3.0, 10.0);
    let evicted = budget.set_max_voices(2, 4.0);
    assert_eq!(
        evicted,
        vec![2, 3],
        "lowest priority first, oldest among equals"
    );
    assert_eq!(budget.ids(4.0), vec![1, 4]);
    assert_eq!(budget.request(5, 1, 4.0, 1.0), VoiceDecision::Reject);
}

#[test]
fn shrinking_does_not_report_voices_that_already_ended() {
    let mut budget = VoiceBudget::new(3);
    budget.request(1, 1, 0.0, 1.0);
    budget.request(2, 1, 0.0, 10.0);
    budget.request(3, 1, 0.0, 10.0);
    let evicted = budget.set_max_voices(2, 5.0);
    assert!(evicted.is_empty(), "voice 1 ended on its own: {evicted:?}");
}

#[test]
fn raising_the_limit_evicts_nothing_and_allows_more_voices() {
    let mut budget = VoiceBudget::new(1);
    budget.request(1, 1, 0.0, 10.0);
    assert!(budget.set_max_voices(3, 0.0).is_empty());
    assert_eq!(budget.request(2, 1, 0.0, 10.0), VoiceDecision::Play);
}

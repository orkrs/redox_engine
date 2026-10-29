//! Adaptive music (AUD-06): layer gains from one control value, smoothed over time.

#![allow(clippy::unwrap_used)]

use redox_audio::music_layers::approach;
use redox_audio::{AdaptiveMusic, AudioContext, MusicLayer};

const TONE: &[u8] = include_bytes!("data/tone.ogg");

fn layer(track: &str, from: f32, full: f32) -> MusicLayer {
    MusicLayer {
        track: track.to_string(),
        fade_in_from: from,
        full_from: full,
    }
}

fn two_layers() -> AdaptiveMusic {
    AdaptiveMusic {
        layers: vec![layer("base", 0.0, 0.0), layer("combat", 0.4, 0.8)],
    }
}

#[test]
fn the_base_layer_is_full_at_zero() {
    let g = two_layers().gains(0.0);
    assert!((g[0] - 1.0).abs() < 1e-6);
    assert_eq!(g[1], 0.0);
}

#[test]
fn a_layer_is_silent_below_its_range_and_full_above_it() {
    let music = two_layers();
    assert_eq!(music.gains(0.2)[1], 0.0);
    assert!((music.gains(0.9)[1] - 1.0).abs() < 1e-6);
}

#[test]
fn a_layer_is_half_power_in_the_middle() {
    let music = AdaptiveMusic {
        layers: vec![layer("x", 0.0, 1.0)],
    };
    let g = music.gains(0.5)[0];
    assert!((g * g - 0.5).abs() < 1e-5, "power {}", g * g);
}

#[test]
fn control_is_clamped_and_nan_is_zero() {
    let music = two_layers();
    assert_eq!(music.gains(-5.0), music.gains(0.0));
    assert_eq!(music.gains(9.0), music.gains(1.0));
    assert_eq!(music.gains(f32::NAN), music.gains(0.0));
}

#[test]
fn equal_edges_switch_on_without_dividing_by_zero() {
    let music = AdaptiveMusic {
        layers: vec![layer("x", 0.5, 0.5)],
    };
    assert_eq!(music.gains(0.49)[0], 0.0);
    assert!((music.gains(0.5)[0] - 1.0).abs() < 1e-6);
    assert!(music.gains(0.7)[0].is_finite());
}

#[test]
fn approach_does_not_overshoot_and_ignores_nan() {
    assert!((approach(0.0, 1.0, 2.0, 0.25) - 0.5).abs() < 1e-6);
    assert_eq!(approach(0.9, 1.0, 2.0, 1.0), 1.0);
    assert_eq!(approach(0.3, 1.0, f32::NAN, 1.0), 0.3);
    assert_eq!(approach(0.3, 1.0, 2.0, f32::NAN), 0.3);
}

#[test]
fn adaptive_music_without_a_device_does_not_panic() {
    let mut ctx = AudioContext::new();
    ctx.play_adaptive_music(two_layers());
    ctx.set_music_control(0.7);
    ctx.update_music(0.1);
    ctx.stop_adaptive_music();
    ctx.update_music(5.0);
}

#[test]
fn an_unregistered_layer_is_skipped() {
    let mut ctx = AudioContext::new();
    if !ctx.is_available() {
        return;
    }
    ctx.register_music("base", TONE.to_vec()).unwrap();
    ctx.play_adaptive_music(two_layers());
    assert_eq!(ctx.playing_music_voices(), 1);
}

#[test]
fn with_a_device_all_registered_layers_start_together() {
    let mut ctx = AudioContext::new();
    if !ctx.is_available() {
        return;
    }
    ctx.register_music("base", TONE.to_vec()).unwrap();
    ctx.register_music("combat", TONE.to_vec()).unwrap();
    ctx.play_adaptive_music(two_layers());
    assert_eq!(ctx.playing_music_voices(), 2);
}

#[test]
fn stopping_adaptive_music_removes_the_voices_after_the_fade() {
    let mut ctx = AudioContext::new();
    if !ctx.is_available() {
        return;
    }
    ctx.register_music("base", TONE.to_vec()).unwrap();
    ctx.register_music("combat", TONE.to_vec()).unwrap();
    ctx.play_adaptive_music(two_layers());
    ctx.stop_adaptive_music();
    ctx.update_music(5.0);
    assert_eq!(ctx.playing_music_voices(), 0);
}

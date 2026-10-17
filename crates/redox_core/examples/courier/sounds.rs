//! Sound effects of Courier: short tones synthesised at start-up (the project ships no audio
//! files) and the cue table that maps game events to them.

use std::f32::consts::TAU;

use kira::Frame;
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use redox_audio::AudioContext;

/// Sample rate of the generated sounds.
pub const SAMPLE_RATE: u32 = 44_100;
/// Fade in and out, so a tone never starts or ends with a click.
const FADE_SECONDS: f32 = 0.008;

/// Cue table (RON): game event -> sound variants, volume, pitch, cooldown, bus and priority.
pub const CUES: &str = r#"[
    (name: "coin", variants: ["coin_a", "coin_b"], volume: (0.7, 0.9), pitch: (0.95, 1.1),
     cooldown: 0.05, bus: Sfx, priority: 6),
    (name: "wave", variants: ["wave"], volume: (0.6, 0.6), pitch: (0.97, 1.03),
     cooldown: 0.3, bus: Sfx, priority: 3),
    (name: "win", variants: ["win"], volume: (0.9, 0.9), pitch: (1.0, 1.0),
     cooldown: 1.0, bus: Sfx, priority: 9),
    (name: "select", variants: ["select"], volume: (0.5, 0.5), pitch: (1.0, 1.0),
     cooldown: 0.05, bus: Ui, priority: 4),
]"#;

/// A tone that glides from `from_hz` to `to_hz` over `seconds`, with a soft second harmonic.
pub fn tone(from_hz: f32, to_hz: f32, seconds: f32, volume: f32) -> Vec<Frame> {
    let count = (seconds.max(0.0) * SAMPLE_RATE as f32) as usize;
    let mut phase = 0.0f32;
    (0..count)
        .map(|i| {
            let t = i as f32 / count.max(1) as f32;
            let hz = from_hz + (to_hz - from_hz) * t;
            phase = (phase + hz / SAMPLE_RATE as f32).fract();
            let time = i as f32 / SAMPLE_RATE as f32;
            let fade_in = (time / FADE_SECONDS).min(1.0);
            let fade_out = ((seconds - time) / FADE_SECONDS).clamp(0.0, 1.0);
            let decay = 1.0 - 0.6 * t;
            let sample = (phase * TAU).sin() * 0.8 + (phase * 2.0 * TAU).sin() * 0.2;
            let value = sample * volume * fade_in.min(fade_out) * decay;
            Frame {
                left: value,
                right: value,
            }
        })
        .collect()
}

/// Several tones one after another.
fn arpeggio(notes: &[(f32, f32)], volume: f32) -> Vec<Frame> {
    notes
        .iter()
        .flat_map(|(hz, seconds)| tone(*hz, *hz, *seconds, volume))
        .collect()
}

fn data(frames: Vec<Frame>) -> StaticSoundData {
    StaticSoundData {
        sample_rate: SAMPLE_RATE,
        frames: frames.into(),
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

/// The generated sounds by the names the cue table uses.
pub fn all() -> Vec<(&'static str, Vec<Frame>)> {
    vec![
        ("coin_a", tone(880.0, 1320.0, 0.16, 0.8)),
        ("coin_b", tone(988.0, 1480.0, 0.16, 0.8)),
        ("wave", tone(440.0, 560.0, 0.25, 0.7)),
        (
            "win",
            arpeggio(
                &[(523.0, 0.14), (659.0, 0.14), (784.0, 0.14), (1046.0, 0.4)],
                0.8,
            ),
        ),
        ("select", tone(660.0, 660.0, 0.07, 0.6)),
    ]
}

/// Registers every generated sound in the audio context.
pub fn register_all(audio: &mut AudioContext) {
    for (name, frames) in all() {
        audio.register_sfx_data(name, data(frames));
    }
}

#[cfg(test)]
mod tests {
    use redox_audio::CueLibrary;

    use super::*;

    #[test]
    fn a_tone_has_the_requested_length_and_never_clips() {
        let frames = tone(440.0, 880.0, 0.25, 0.8);
        assert_eq!(frames.len(), (0.25 * SAMPLE_RATE as f32) as usize);
        let peak = frames.iter().map(|f| f.left.abs()).fold(0.0, f32::max);
        assert!(peak > 0.2 && peak <= 0.8 + 1e-3, "peak {peak}");
        assert!(frames.iter().all(|f| f.left == f.right));
    }

    #[test]
    fn a_tone_starts_and_ends_silent() {
        let frames = tone(500.0, 500.0, 0.2, 1.0);
        assert!(frames[0].left.abs() < 0.05);
        assert!(frames.last().unwrap().left.abs() < 0.05);
    }

    #[test]
    fn a_zero_length_tone_is_empty() {
        assert!(tone(440.0, 440.0, 0.0, 1.0).is_empty());
        assert!(tone(440.0, 440.0, -1.0, 1.0).is_empty());
    }

    #[test]
    fn every_cue_variant_has_a_generated_sound() {
        let library = CueLibrary::parse(CUES).unwrap();
        let names: Vec<_> = all().into_iter().map(|(name, _)| name).collect();
        for cue in ["coin", "wave", "win", "select"] {
            let cue = library
                .get(cue)
                .unwrap_or_else(|| panic!("cue {cue} missing"));
            for variant in &cue.variants {
                assert!(names.contains(&variant.as_str()), "no sound for {variant}");
            }
        }
        assert_eq!(library.len(), 4);
    }

    #[test]
    fn registering_works_without_an_audio_device() {
        let mut audio = AudioContext::new();
        register_all(&mut audio); // must not panic
    }
}

//! Sound effects of Foxfire: synthesised tones (no audio files) and the cue table.

use kira::Frame;
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use redox_audio::AudioContext;

// The tone generator is shared with the Courier example.
#[allow(dead_code)]
#[path = "../courier/sounds.rs"]
mod base;

use base::{SAMPLE_RATE, tone};

pub const CUES: &str = r#"[
    (name: "gem", variants: ["gem_a", "gem_b", "gem_c"], volume: (0.7, 0.85), pitch: (0.95, 1.15),
     cooldown: 0.04, bus: Sfx, priority: 6),
    (name: "hit", variants: ["hit"], volume: (0.9, 0.9), pitch: (0.95, 1.05),
     cooldown: 0.3, bus: Sfx, priority: 9),
    (name: "win", variants: ["win"], volume: (0.9, 0.9), pitch: (1.0, 1.0),
     cooldown: 1.0, bus: Sfx, priority: 9),
    (name: "lose", variants: ["lose"], volume: (0.9, 0.9), pitch: (1.0, 1.0),
     cooldown: 1.0, bus: Sfx, priority: 9),
    (name: "select", variants: ["select"], volume: (0.5, 0.5), pitch: (1.0, 1.0),
     cooldown: 0.05, bus: Ui, priority: 4),
    (name: "exhausted", variants: ["exhausted"], volume: (0.5, 0.5), pitch: (1.0, 1.0),
     cooldown: 1.0, bus: Sfx, priority: 5),
]"#;

fn notes(list: &[(f32, f32)], volume: f32) -> Vec<Frame> {
    list.iter()
        .flat_map(|(hz, seconds)| tone(*hz, *hz, *seconds, volume))
        .collect()
}

/// The generated sounds by the names the cue table uses.
pub fn all() -> Vec<(&'static str, Vec<Frame>)> {
    vec![
        ("gem_a", tone(1046.0, 1568.0, 0.14, 0.7)),
        ("gem_b", tone(1175.0, 1760.0, 0.14, 0.7)),
        ("gem_c", tone(1318.0, 1976.0, 0.14, 0.7)),
        ("hit", tone(220.0, 90.0, 0.3, 0.9)),
        (
            "win",
            notes(
                &[
                    (523.0, 0.12),
                    (659.0, 0.12),
                    (784.0, 0.12),
                    (1046.0, 0.12),
                    (1318.0, 0.45),
                ],
                0.8,
            ),
        ),
        (
            "lose",
            notes(&[(392.0, 0.2), (330.0, 0.2), (262.0, 0.5)], 0.8),
        ),
        ("select", tone(660.0, 660.0, 0.07, 0.6)),
        ("exhausted", tone(300.0, 200.0, 0.25, 0.6)),
    ]
}

pub fn register_all(audio: &mut AudioContext) {
    for (name, frames) in all() {
        audio.register_sfx_data(
            name,
            StaticSoundData {
                sample_rate: SAMPLE_RATE,
                frames: frames.into(),
                settings: StaticSoundSettings::default(),
                slice: None,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use redox_audio::CueLibrary;

    use super::*;

    #[test]
    fn every_cue_variant_has_a_sound() {
        let library = CueLibrary::parse(CUES).unwrap();
        let names: Vec<_> = all().into_iter().map(|(n, _)| n).collect();
        for cue in ["gem", "hit", "win", "lose", "select", "exhausted"] {
            let cue = library.get(cue).unwrap();
            for variant in &cue.variants {
                assert!(names.contains(&variant.as_str()), "no sound for {variant}");
            }
        }
    }

    #[test]
    fn registering_works_without_a_device() {
        register_all(&mut AudioContext::new());
    }
}

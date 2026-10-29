//! Measurements of sound files for authoring checks (AUD-07, step 2).
//!
//! [`analyze_samples`] measures interleaved samples: peak and RMS levels, integrated loudness
//! (ITU-R BS.1770 with K-weighting and gating), clipping, DC offset and duration.
//! [`loop_seam_error`] compares the level at the start and at the end of a looped clip.
//!
//! [`decode_bytes`] decodes a file with kira's decoder, which is already a dependency. kira always
//! produces stereo frames, so [`frames_to_samples`] reports a file whose two channels are identical
//! at every frame as mono. Without this a mono file would read about 3 dB too loud.

use std::io::Cursor;

use kira::Frame;
use kira::sound::static_sound::StaticSoundData;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Floor of every level in dB. A zero signal, and any level below the floor, reads as this value.
pub const SILENCE_DB: f32 = -120.0;
/// Absolute gate of BS.1770, in LUFS.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Relative gate of BS.1770, in LU below the ungated mean loudness.
const RELATIVE_GATE_LU: f64 = -10.0;
/// Length of one loudness block, in seconds.
const BLOCK_SECONDS: f64 = 0.4;
/// Distance between the starts of two loudness blocks (75% overlap), in seconds.
const BLOCK_STEP_SECONDS: f64 = 0.1;
/// Constant offset of the BS.1770 loudness formula.
const LOUDNESS_OFFSET: f64 = -0.691;
/// Level at which a sample counts as clipped.
const CLIP_LEVEL: f32 = 1.0;

// K-weighting coefficients (ITU-R BS.1770-4), derived from the analog prototypes for any sample rate.
const SHELF_F0: f64 = 1_681.974_450_955_533;
const SHELF_GAIN_DB: f64 = 3.999_843_853_973_347;
const SHELF_Q: f64 = 0.707_175_236_955_419_6;
const SHELF_VB_EXPONENT: f64 = 0.499_666_774_154_541_6;
const HIGHPASS_F0: f64 = 38.135_470_876_024_44;
const HIGHPASS_Q: f64 = 0.500_327_037_323_877_3;

/// Measured properties of one clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioStats {
    /// Highest absolute sample in dBFS, floored at `SILENCE_DB`.
    pub peak_db: f32,
    /// RMS level over all samples in dBFS, floored at `SILENCE_DB`.
    pub rms_db: f32,
    /// Integrated loudness in LUFS (BS.1770, gated), floored at `SILENCE_DB`. A clip where no
    /// block passes the gate gets the floor.
    pub lufs_integrated: f32,
    /// Length in seconds.
    pub duration: f32,
    /// Number of samples at or above full scale.
    pub clipped_samples: usize,
    /// Mean sample value (DC offset).
    pub dc_offset: f32,
    /// Channel count the loudness was measured with (1 for a mono file).
    pub channels_detected: u16,
}

impl AudioStats {
    /// True when the RMS level is at the floor, that is the clip is silent.
    pub fn is_silent(&self) -> bool {
        self.rms_db <= SILENCE_DB
    }
}

/// Failure to decode a sound file.
#[derive(Debug, Error)]
pub enum AnalyzeError {
    /// The bytes are not a sound file the decoder understands.
    #[error("audio could not be decoded: {0}")]
    Decode(String),
}

/// Interleaved samples of a decoded file with their channel count and sample rate.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAudio {
    /// Interleaved samples. One channel for a mono file, two otherwise.
    pub samples: Vec<f32>,
    /// Number of channels in `samples`.
    pub channels: u16,
    /// Sample rate in Hz.
    pub sample_rate: u32,
}

/// Measures interleaved `samples` with `channels` channels at `rate` Hz.
///
/// Channels beyond the first two are weighted like the others. The loudness of a file that
/// is shorter than one BS.1770 block is taken over the whole file.
pub fn analyze_samples(samples: &[f32], channels: u16, rate: u32) -> AudioStats {
    let ch = usize::from(channels.max(1));
    let frames = samples.len() / ch;
    let count = samples.len() as f64;
    let sum: f64 = samples.iter().map(|&s| f64::from(s)).sum();
    let sum_sq: f64 = samples.iter().map(|&s| f64::from(s).powi(2)).sum();
    let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    let per_channel: Vec<Vec<f64>> = (0..ch)
        .map(|c| {
            samples
                .iter()
                .skip(c)
                .step_by(ch)
                .map(|&s| f64::from(s))
                .collect()
        })
        .collect();
    let weighted: Vec<Vec<f64>> = per_channel
        .iter()
        .map(|channel| k_weight(channel, rate))
        .collect();
    AudioStats {
        peak_db: amplitude_db(f64::from(peak)),
        rms_db: if count > 0.0 {
            amplitude_db((sum_sq / count).sqrt())
        } else {
            SILENCE_DB
        },
        lufs_integrated: floor_db(integrated_lufs(&weighted, rate)),
        duration: frames as f32 / rate.max(1) as f32,
        clipped_samples: samples.iter().filter(|s| s.abs() >= CLIP_LEVEL).count(),
        dc_offset: if count > 0.0 {
            (sum / count) as f32
        } else {
            0.0
        },
        channels_detected: channels.max(1),
    }
}

/// Measures the seam of a loop: the difference in dB between the RMS level of the last `window`
/// frames and the RMS level of the first `window` frames.
///
/// A perfect loop gives about 0 dB. A clip that ends louder than it starts gives a large value.
/// The measure compares levels only, so a phase jump between two equally loud ends is not seen.
/// Both levels are floored at `SILENCE_DB`, so a silent end gives a finite value.
/// `window` is limited to half the clip. Returns 0 for an empty clip or a zero window.
pub fn loop_seam_error(samples: &[f32], channels: u16, window: usize) -> f32 {
    let ch = usize::from(channels.max(1));
    let frames = samples.len() / ch;
    let window = window.min(frames / 2);
    if window == 0 {
        return 0.0;
    }
    let head_db = amplitude_db(rms(&samples[..window * ch]));
    let tail_db = amplitude_db(rms(&samples[(frames - window) * ch..frames * ch]));
    (head_db - tail_db).abs()
}

/// Decodes a sound file (WAV or OGG, whatever the kira decoder supports) into samples.
///
/// # Errors
/// Returns [`AnalyzeError::Decode`] when the bytes are not a readable sound file.
pub fn decode_bytes(bytes: &[u8]) -> Result<DecodedAudio, AnalyzeError> {
    let data = StaticSoundData::from_cursor(Cursor::new(bytes.to_vec()))
        .map_err(|error| AnalyzeError::Decode(error.to_string()))?;
    let (samples, channels) = frames_to_samples(&data.frames);
    Ok(DecodedAudio {
        samples,
        channels,
        sample_rate: data.sample_rate,
    })
}

/// Converts stereo frames to samples. Returns one channel when left and right are equal at every
/// frame (a mono source), and two interleaved channels otherwise.
pub fn frames_to_samples(frames: &[Frame]) -> (Vec<f32>, u16) {
    if frames.iter().all(|frame| frame.left == frame.right) {
        (frames.iter().map(|frame| frame.left).collect(), 1)
    } else {
        (
            frames
                .iter()
                .flat_map(|frame| [frame.left, frame.right])
                .collect(),
            2,
        )
    }
}

/// Converts a linear amplitude to dB, floored at `SILENCE_DB`. Zero (or less) gives the floor.
fn amplitude_db(amplitude: f64) -> f32 {
    if amplitude <= 0.0 {
        SILENCE_DB
    } else {
        floor_db(20.0 * amplitude.log10())
    }
}

/// Clamps a dB value to `SILENCE_DB`. A non-finite value (no gated block) gives the floor.
fn floor_db(db: f64) -> f32 {
    if db.is_finite() {
        db.max(f64::from(SILENCE_DB)) as f32
    } else {
        SILENCE_DB
    }
}

/// Root mean square of a slice. An empty slice gives 0.
fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| f64::from(s).powi(2)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// One second-order IIR section with the leading denominator coefficient fixed at 1.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f64; 3],
    /// Denominator coefficients `a1` and `a2` (`a0` is 1).
    a: [f64; 2],
}

impl Biquad {
    /// The K-weighting high shelf (stage 1 of BS.1770).
    fn shelf(rate: f64) -> Self {
        let k = (std::f64::consts::PI * SHELF_F0 / rate).tan();
        let vh = 10f64.powf(SHELF_GAIN_DB / 20.0);
        let vb = vh.powf(SHELF_VB_EXPONENT);
        let a0 = 1.0 + k / SHELF_Q + k * k;
        Self {
            b: [
                (vh + vb * k / SHELF_Q + k * k) / a0,
                2.0 * (k * k - vh) / a0,
                (vh - vb * k / SHELF_Q + k * k) / a0,
            ],
            a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / SHELF_Q + k * k) / a0],
        }
    }

    /// The K-weighting high-pass (stage 2 of BS.1770).
    fn highpass(rate: f64) -> Self {
        let k = (std::f64::consts::PI * HIGHPASS_F0 / rate).tan();
        let a0 = 1.0 + k / HIGHPASS_Q + k * k;
        Self {
            b: [1.0 / a0, -2.0 / a0, 1.0 / a0],
            a: [
                2.0 * (k * k - 1.0) / a0,
                (1.0 - k / HIGHPASS_Q + k * k) / a0,
            ],
        }
    }

    /// Filters a whole channel, starting from silent state.
    fn run(&self, input: &[f64]) -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        input
            .iter()
            .map(|&x| {
                let y = self.b[0] * x + self.b[1] * x1 + self.b[2] * x2
                    - self.a[0] * y1
                    - self.a[1] * y2;
                x2 = x1;
                x1 = x;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    }
}

/// Applies the two K-weighting stages to one channel.
fn k_weight(channel: &[f64], rate: u32) -> Vec<f64> {
    let rate = f64::from(rate.max(1));
    Biquad::highpass(rate).run(&Biquad::shelf(rate).run(channel))
}

/// BS.1770 loudness of a mean power (sum of weighted channel powers).
fn loudness_of_power(power: f64) -> f64 {
    LOUDNESS_OFFSET + 10.0 * power.log10()
}

/// Integrated loudness of K-weighted channels, with absolute and relative gating.
fn integrated_lufs(weighted: &[Vec<f64>], rate: u32) -> f64 {
    let frames = weighted.first().map_or(0, Vec::len);
    if frames == 0 {
        return f64::NEG_INFINITY;
    }
    let rate = f64::from(rate.max(1));
    let block = ((BLOCK_SECONDS * rate).round() as usize).max(1);
    let step = ((BLOCK_STEP_SECONDS * rate).round() as usize).max(1);
    let powers: Vec<f64> = if frames < block {
        vec![block_power(weighted, 0, frames)]
    } else {
        (0..=frames - block)
            .step_by(step)
            .map(|start| block_power(weighted, start, start + block))
            .collect()
    };
    gated_loudness(&powers)
}

/// Mean square of every channel over `start..end`, summed over channels.
fn block_power(weighted: &[Vec<f64>], start: usize, end: usize) -> f64 {
    let len = (end - start) as f64;
    weighted
        .iter()
        .map(|channel| channel[start..end].iter().map(|x| x * x).sum::<f64>() / len)
        .sum()
}

/// Applies the absolute and relative gates to block powers and returns the gated loudness.
fn gated_loudness(powers: &[f64]) -> f64 {
    let absolute: Vec<f64> = powers
        .iter()
        .copied()
        .filter(|&p| loudness_of_power(p) > ABSOLUTE_GATE_LUFS)
        .collect();
    if absolute.is_empty() {
        return f64::NEG_INFINITY;
    }
    let threshold = loudness_of_power(mean(&absolute)) + RELATIVE_GATE_LU;
    let kept: Vec<f64> = absolute
        .into_iter()
        .filter(|&p| loudness_of_power(p) > threshold)
        .collect();
    if kept.is_empty() {
        f64::NEG_INFINITY
    } else {
        loudness_of_power(mean(&kept))
    }
}

/// Arithmetic mean of a non-empty slice.
fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A sine of `amplitude` at `freq` Hz for `seconds`, sampled at [`RATE`].
    fn sine(freq: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        let count = (RATE as f32 * seconds) as usize;
        (0..count)
            .map(|n| {
                let t = n as f64 / f64::from(RATE);
                (amplitude as f64 * (2.0 * std::f64::consts::PI * f64::from(freq) * t).sin()) as f32
            })
            .collect()
    }

    /// Amplitude of a sine whose RMS level is `rms_dbfs`.
    fn amplitude_for_rms_db(rms_dbfs: f32) -> f32 {
        std::f32::consts::SQRT_2 * 10f32.powf(rms_dbfs / 20.0)
    }

    #[test]
    fn a_full_scale_sine_peaks_at_zero_dbfs() {
        let stats = analyze_samples(&sine(1000.0, 1.0, 1.0), 1, RATE);
        assert!((stats.peak_db - 0.0).abs() < 0.01, "peak {}", stats.peak_db);
    }

    #[test]
    fn a_minus_20_dbfs_sine_has_the_expected_rms() {
        // Peak of 0.1 is -20 dBFS; its RMS is -23.01 dBFS.
        let stats = analyze_samples(&sine(1000.0, 0.1, 1.0), 1, RATE);
        assert!((stats.rms_db - -23.0).abs() < 0.1, "rms {}", stats.rms_db);
    }

    #[test]
    fn lufs_of_a_997hz_sine_at_minus_23_dbfs_is_minus_23_lufs_within_0_2() {
        // The level is an RMS level of -23 dBFS (a peak of -20 dBFS). The K-weighting filter adds
        // about +0.65 dB at 997 Hz, so BS.1770 gives -23.05 LUFS for a mono signal.
        let samples = sine(997.0, amplitude_for_rms_db(-23.0), 10.0);
        let stats = analyze_samples(&samples, 1, RATE);
        assert!(
            (stats.lufs_integrated - -23.0).abs() <= 0.2,
            "lufs {}",
            stats.lufs_integrated
        );
    }

    #[test]
    fn silence_is_flagged() {
        let silent = vec![0.0_f32; RATE as usize];
        let stats = analyze_samples(&silent, 1, RATE);
        assert!(stats.is_silent());
        assert_eq!(stats.lufs_integrated, SILENCE_DB);
        assert_eq!(stats.peak_db, SILENCE_DB);
        assert_eq!(stats.rms_db, SILENCE_DB);
    }

    #[test]
    fn clipped_samples_are_counted() {
        let stats = analyze_samples(&[0.5, -1.0, 1.0, 0.99, 1.2], 1, RATE);
        assert_eq!(stats.clipped_samples, 3);
    }

    #[test]
    fn a_perfect_loop_has_a_near_zero_seam_error() {
        // 1 kHz at 48 kHz repeats every 48 frames, so 1 s is exactly 1000 periods.
        let samples = sine(1000.0, 0.5, 1.0);
        let error = loop_seam_error(&samples, 1, 480);
        assert!(error.abs() < 0.01, "seam {error}");
    }

    #[test]
    fn a_hard_cut_has_a_large_seam_error() {
        // The clip ends at 0.8 and starts at 0.1: a loud jump at the seam.
        let mut samples = sine(1000.0, 0.1, 0.5);
        samples.extend(sine(1000.0, 0.8, 0.5));
        let error = loop_seam_error(&samples, 1, 480);
        assert!(error > 3.0, "seam {error}");
    }

    #[test]
    fn a_silent_end_gives_a_finite_seam_error() {
        let mut samples = sine(1000.0, 0.5, 0.5);
        samples.extend(std::iter::repeat_n(0.0_f32, samples.len()));
        let error = loop_seam_error(&samples, 1, 480);
        assert!(error.is_finite() && error > 3.0, "seam {error}");
    }

    #[test]
    fn a_duplicated_mono_signal_is_analysed_as_mono() {
        let mono = sine(997.0, amplitude_for_rms_db(-23.0), 10.0);
        let frames: Vec<Frame> = mono.iter().map(|&s| Frame { left: s, right: s }).collect();
        let (samples, channels) = frames_to_samples(&frames);
        assert_eq!(channels, 1);
        assert_eq!(samples.len(), mono.len());
        let stats = analyze_samples(&samples, channels, RATE);
        assert_eq!(stats.channels_detected, 1);
        assert!((stats.lufs_integrated - -23.0).abs() <= 0.2);
    }

    #[test]
    fn a_stereo_signal_keeps_two_channels() {
        let frames = [
            Frame {
                left: 0.5,
                right: 0.25,
            },
            Frame {
                left: 0.1,
                right: 0.0,
            },
        ];
        let (samples, channels) = frames_to_samples(&frames);
        assert_eq!(channels, 2);
        assert_eq!(samples, vec![0.5, 0.25, 0.1, 0.0]);
    }

    #[test]
    fn undecodable_bytes_are_a_decode_error() {
        let result = decode_bytes(b"not a sound file");
        assert!(matches!(result, Err(AnalyzeError::Decode(_))));
    }
}

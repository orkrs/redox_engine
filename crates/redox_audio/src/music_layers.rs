//! Adaptive music layers: several stems of one theme play together, and a single control value
//! (0..1, for example "tension") moves each stem's volume.
//!
//! This module is pure logic with no audio device. [`AdaptiveMusic::gains`] maps the control value
//! to the gain of every layer, and [`approach`] smooths a gain over time so that a sudden jump of
//! the control value does not make the volume jump.

use std::f32::consts::FRAC_PI_2;

/// One stem of an adaptive track and the range of the control value where it is audible.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicLayer {
    /// Name of a registered music track (see `register_music`).
    pub track: String,
    /// The control value at which the layer starts to fade in (0..1).
    pub fade_in_from: f32,
    /// The control value at which the layer is fully in (>= `fade_in_from`).
    pub full_from: f32,
}

/// A set of layers driven by one control value. Layers that start at 0 are the always-on base.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AdaptiveMusic {
    /// The layers, in the order their gains are reported.
    pub layers: Vec<MusicLayer>,
}

impl AdaptiveMusic {
    /// Linear gain of every layer (same order as `layers`) for `control` in 0..1.
    ///
    /// The control value is clamped to 0..1 and NaN counts as 0. A layer whose `fade_in_from` is not
    /// below `full_from` switches on at `full_from`, with no division by zero. Gains are in 0..=1 and
    /// follow an equal-power curve `sin(t * PI / 2)`, where `t` is the position in the fade range.
    pub fn gains(&self, control: f32) -> Vec<f32> {
        let control = if control.is_nan() {
            0.0
        } else {
            control.clamp(0.0, 1.0)
        };
        self.layers
            .iter()
            .map(|layer| layer_gain(layer, control))
            .collect()
    }
}

/// Gain of one layer for a control value that is already clamped to 0..1.
fn layer_gain(layer: &MusicLayer, control: f32) -> f32 {
    let span = layer.full_from - layer.fade_in_from;
    let t = if span > 0.0 {
        ((control - layer.fade_in_from) / span).clamp(0.0, 1.0)
    } else if control >= layer.full_from {
        1.0
    } else {
        0.0
    };
    (t * FRAC_PI_2).sin()
}

/// Moves `current` toward `target` by at most `rate * dt` (`rate` in gain units per second).
///
/// The result never passes `target`. A NaN `dt` or `rate` leaves `current` unchanged, and a
/// negative step is treated as zero.
pub fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    if rate.is_nan() || dt.is_nan() {
        return current;
    }
    let step = (rate * dt).max(0.0);
    let delta = target - current;
    if delta.abs() <= step {
        target
    } else {
        current + step.copysign(delta)
    }
}

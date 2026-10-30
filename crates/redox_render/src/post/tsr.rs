//! Temporal upscaling (TSR) math for `render_scale < 1` (PST-07).
//!
//! These helpers are the CPU side of the TAA upscale path: jitter sequence, texture LOD bias,
//! sharpening amount and the history-rejection scale. They mirror `shaders/taa.wgsl`.
//! At `render_scale >= 0.999` every helper returns its native value, so the native TAA frame
//! keeps the legacy 16-phase jitter and stays bitwise unchanged.

use crate::pass::taa::{halton, halton_jitter_ndc};

/// Scales at or above this value take the native (non-upscaling) path.
pub const NATIVE_SCALE_THRESHOLD: f32 = 0.999;
/// Smallest jitter cycle; the phase count never drops below it.
pub const MIN_JITTER_PHASES: u32 = 8;
/// Most negative texture LOD bias (sampler mip selection).
pub const MIN_LOD_BIAS: f32 = -2.0;
/// Sharpening amount reached at render scale 0.5 (see [`sharpen_amount`]).
pub const MAX_SHARPEN_AMOUNT: f32 = 0.35;

/// Returns `true` when `render_scale` is native (no upscaling).
pub fn is_native(render_scale: f32) -> bool {
    render_scale >= NATIVE_SCALE_THRESHOLD
}

/// Number of jitter phases for `render_scale`: `ceil(8 / scale²)`, never below 8.
///
/// Lower scales need more sub-pixel samples per output pixel, so the cycle gets longer.
pub fn jitter_phase_count(render_scale: f32) -> u32 {
    let s = render_scale.max(f32::EPSILON);
    ((8.0 / (s * s)).ceil() as u32).max(MIN_JITTER_PHASES)
}

/// Halton(2,3) jitter for frame `index` in **input pixels**, centred on zero, in `(-0.5, 0.5)`.
///
/// The cycle length is [`jitter_phase_count`], so a cycle never repeats an offset.
pub fn jitter_sequence(index: u32, render_scale: f32) -> [f32; 2] {
    let phases = jitter_phase_count(render_scale);
    let i = u64::from(index % phases) + 1;
    [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
}

/// Jitter in NDC for the current frame.
///
/// Native scale uses the legacy 16-phase path ([`halton_jitter_ndc`]) so that frames stay
/// bitwise identical. Upscaled frames use [`jitter_sequence`] converted with the input size.
pub fn jitter_ndc(
    frame_index: u64,
    render_scale: f32,
    input_width: u32,
    input_height: u32,
) -> [f32; 2] {
    if is_native(render_scale) {
        return halton_jitter_ndc(frame_index, input_width, input_height);
    }
    let px = jitter_sequence(frame_index as u32, render_scale);
    [
        px[0] * 2.0 / input_width as f32,
        px[1] * 2.0 / input_height as f32,
    ]
}

/// Texture LOD bias for material samplers: `log2(scale) - 1`, clamped to `-2`.
///
/// Zero at native scale. Negative below native so that textures stay as sharp as at native.
pub fn texture_lod_bias(render_scale: f32) -> f32 {
    if is_native(render_scale) {
        return 0.0;
    }
    (render_scale.log2() - 1.0).max(MIN_LOD_BIAS)
}

/// Sharpening amount: `0` at native, rising linearly to [`MAX_SHARPEN_AMOUNT`] at scale 0.5.
pub fn sharpen_amount(render_scale: f32) -> f32 {
    if is_native(render_scale) {
        return 0.0;
    }
    let t = ((1.0 - render_scale) / 0.5).clamp(0.0, 1.0);
    MAX_SHARPEN_AMOUNT * t
}

/// Scale of the history clip window and depth-rejection threshold: `1 / scale` when upscaling.
///
/// Returns `1.0` at native scale, so the TAA shader arithmetic is unchanged there.
pub fn inv_scale(render_scale: f32) -> f32 {
    if is_native(render_scale) {
        return 1.0;
    }
    1.0 / render_scale
}

/// RCAS sharpness stops for the final pass: the base value minus [`sharpen_amount`].
///
/// Lower stops mean a stronger sharpen. Native scale returns `base` unchanged.
pub fn rcas_stops(render_scale: f32, base_stops: f32) -> f32 {
    if is_native(render_scale) {
        return base_stops;
    }
    (base_stops - sharpen_amount(render_scale)).max(0.0)
}

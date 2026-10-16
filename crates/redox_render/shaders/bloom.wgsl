//! Dual-filter bloom — 6-mip downsample / upsample fullscreen passes.
//!
//! `fs_downsample_threshold` extracts bright HDR into the first mip.
//! `fs_downsample` continues the chain without re-thresholding.
//! `fs_upsample` applies a 9-tap tent filter (caller blends ADD into the
//! already-downsampled finer mip).

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_linear: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    out.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

fn sample_box5(uv: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
    var c = textureSample(t_source, s_linear, uv).rgb;
    c = c + textureSample(t_source, s_linear, uv + vec2<f32>( texel.x,  0.0)).rgb;
    c = c + textureSample(t_source, s_linear, uv + vec2<f32>(-texel.x,  0.0)).rgb;
    c = c + textureSample(t_source, s_linear, uv + vec2<f32>( 0.0,  texel.y)).rgb;
    c = c + textureSample(t_source, s_linear, uv + vec2<f32>( 0.0, -texel.y)).rgb;
    return c * 0.2;
}

fn apply_threshold(c: vec3<f32>) -> vec3<f32> {
    if (any(c != c) || any(abs(c) > vec3<f32>(65504.0))) {
        return vec3<f32>(0.0);
    }
    let safe_c = max(c, vec3<f32>(0.0));
    let lum = dot(safe_c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let knee = max(lum - 1.2, 0.0);
    return safe_c * (knee / max(lum, 1.0e-4));
}

@fragment
fn fs_downsample_threshold(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
    let c = apply_threshold(sample_box5(in.uv, texel));
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
    let c = sample_box5(in.uv, texel);
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_upsample(in: VsOut) -> @location(0) vec4<f32> {
    // 9-tap tent filter (weights sum to 16).
    let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
    let o = texel;
    var c = textureSample(t_source, s_linear, in.uv).rgb * 4.0;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>(-o.x,  0.0)).rgb * 2.0;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>( o.x,  0.0)).rgb * 2.0;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>( 0.0, -o.y)).rgb * 2.0;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>( 0.0,  o.y)).rgb * 2.0;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>(-o.x, -o.y)).rgb;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>( o.x, -o.y)).rgb;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>(-o.x,  o.y)).rgb;
    c = c + textureSample(t_source, s_linear, in.uv + vec2<f32>( o.x,  o.y)).rgb;
    c = c * (1.0 / 16.0) * 1.15;
    return vec4<f32>(c, 1.0);
}

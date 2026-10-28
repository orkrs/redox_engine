// Motion blur over the TAA output. Mirrors `motion_blur_offsets` in src/post/post_fx.rs;
// keep the two in sync. Velocity is read at the pixel (UV offset per frame).

struct MotionBlurUniform {
    screen     : vec2<f32>,
    shutter    : f32,
    max_pixels : f32,
    taps       : u32,
    _pad0      : u32,
    _pad1      : u32,
    _pad2      : u32,
};

@group(0) @binding(0) var t_color    : texture_2d<f32>;
@group(0) @binding(1) var t_velocity : texture_2d<f32>;
@group(0) @binding(2) var s_linear   : sampler;
@group(0) @binding(3) var<uniform>     mb : MotionBlurUniform;

struct VsOut {
    @builtin(position) clip_pos : vec4<f32>,
    @location(0)       uv       : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    out.uv       = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let center = textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    let vel = textureSampleLevel(t_velocity, s_linear, in.uv, 0.0).xy;
    let v0 = vel * mb.shutter;
    let len_px = length(v0 * mb.screen);
    // Same rule as the Rust side: blurs under half a pixel leave the pixel sharp.
    if (len_px < 0.5) {
        return center;
    }
    var v = v0;
    if (len_px > mb.max_pixels) {
        v = v0 * (mb.max_pixels / len_px);
    }
    let n = mb.taps;
    var sum = vec4<f32>(0.0);
    for (var i = 0u; i < n; i = i + 1u) {
        let t = f32(i) / f32(n - 1u) - 0.5;
        sum = sum + textureSampleLevel(t_color, s_linear, in.uv + v * t, 0.0);
    }
    return sum / f32(n);
}

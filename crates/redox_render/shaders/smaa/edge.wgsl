// SMAA Edge Detection (Pass 1) — Luma-based, PRESET_HIGH
// Port of iryoku SMAA.hlsl SMAALumaEdgeDetectionPS
//
// Input:  LDR color (post-ACES tonemapped, linear Rgba8Unorm)
// Output: Rg8Unorm edges texture (R = left edge, G = top edge)

struct SmaaMetrics {
    rt_metrics: vec4<f32>, // (1/w, 1/h, w, h)
};

@group(0) @binding(0) var t_color   : texture_2d<f32>;
@group(0) @binding(1) var s_linear  : sampler;
@group(0) @binding(2) var<uniform> u : SmaaMetrics;

const SMAA_THRESHOLD: f32 = 0.05;
const SMAA_LOCAL_CONTRAST_ADAPTATION_FACTOR: f32 = 1.25;
const LUMA_WEIGHTS: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) offset0: vec4<f32>,
    @location(2) offset1: vec4<f32>,
    @location(3) offset2: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    let uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = uv;

    let m = u.rt_metrics;
    out.offset0 = m.xyxy * vec4<f32>(-1.0, 0.0, 0.0, -1.0) + uv.xyxy;
    out.offset1 = m.xyxy * vec4<f32>( 1.0, 0.0, 0.0,  1.0) + uv.xyxy;
    out.offset2 = m.xyxy * vec4<f32>(-2.0, 0.0, 0.0, -2.0) + uv.xyxy;
    return out;
}

fn luma(coord: vec2<f32>) -> f32 {
    return dot(textureSampleLevel(t_color, s_linear, coord, 0.0).rgb, LUMA_WEIGHTS);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec2<f32> {
    let threshold = vec2<f32>(SMAA_THRESHOLD);

    let L      = luma(in.uv);
    let Lleft  = luma(in.offset0.xy);
    let Ltop   = luma(in.offset0.zw);

    let delta_left = abs(L - Lleft);
    let delta_top  = abs(L - Ltop);
    let delta_xy = vec2<f32>(delta_left, delta_top);
    var edges = step(threshold, delta_xy);

    if (dot(edges, vec2<f32>(1.0)) == 0.0) {
        discard;
    }

    let Lright  = luma(in.offset1.xy);
    let Lbottom = luma(in.offset1.zw);
    let delta_right  = abs(L - Lright);
    let delta_bottom = abs(L - Lbottom);

    var maxDelta = max(delta_xy, vec2<f32>(delta_right, delta_bottom));

    let Lleftleft = luma(in.offset2.xy);
    let Ltoptop   = luma(in.offset2.zw);
    let delta_ll = abs(Lleft - Lleftleft);
    let delta_tt = abs(Ltop  - Ltoptop);

    maxDelta = max(maxDelta, vec2<f32>(delta_ll, delta_tt));
    let finalDelta = max(maxDelta.x, maxDelta.y);

    edges *= step(vec2<f32>(finalDelta), SMAA_LOCAL_CONTRAST_ADAPTATION_FACTOR * delta_xy);

    return edges;
}

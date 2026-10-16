// SMAA Neighborhood Blending (Pass 3)
// Port of iryoku SMAA.hlsl SMAANeighborhoodBlendingPS
//
// Input:  LDR color + blend weights texture
// Output: Anti-aliased LDR color (Rgba8Unorm)

struct SmaaMetrics {
    rt_metrics: vec4<f32>, // (1/w, 1/h, w, h)
};

@group(0) @binding(0) var t_color  : texture_2d<f32>;
@group(0) @binding(1) var t_blend  : texture_2d<f32>;
@group(0) @binding(2) var s_linear : sampler;
@group(0) @binding(3) var s_point  : sampler;
@group(0) @binding(4) var<uniform> u : SmaaMetrics;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) offset: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    let uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = uv;
    out.offset = u.rt_metrics.xyxy * vec4<f32>(1.0, 0.0, 0.0, 1.0) + uv.xyxy;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var a: vec4<f32>;
    a.x = textureSampleLevel(t_blend, s_point, in.offset.xy, 0.0).a; // Right
    a.y = textureSampleLevel(t_blend, s_point, in.offset.zw, 0.0).g; // Top
    let local = textureSampleLevel(t_blend, s_point, in.uv, 0.0);
    a.w = local.x; // Bottom (stored in .r => left for vertical pass? Actually .r = left weight from horiz)
    a.z = local.z; // Left

    if (dot(a, vec4<f32>(1.0)) < 1e-5) {
        return textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    }

    let h = max(a.x, a.z) > max(a.y, a.w);

    var blending_offset: vec4<f32>;
    var blending_weight: vec2<f32>;

    if (h) {
        blending_offset = vec4<f32>(a.x, 0.0, a.z, 0.0);
        blending_weight = a.xz;
    } else {
        blending_offset = vec4<f32>(0.0, a.y, 0.0, a.w);
        blending_weight = a.yw;
    }

    blending_weight /= dot(blending_weight, vec2<f32>(1.0, 1.0));

    let blending_coord = blending_offset * vec4<f32>(u.rt_metrics.xy, -u.rt_metrics.xy) + in.uv.xyxy;

    var color = blending_weight.x * textureSampleLevel(t_color, s_linear, blending_coord.xy, 0.0);
    color += blending_weight.y * textureSampleLevel(t_color, s_linear, blending_coord.zw, 0.0);

    return color;
}

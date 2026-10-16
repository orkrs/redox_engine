@group(0) @binding(0) var t_gtao_half: texture_2d<f32>;
@group(0) @binding(1) var s_gtao: sampler;
@group(0) @binding(2) var t_normal_depth: texture_2d<f32>;

const FAR: f32 = 1000.0;
const DEPTH_SIGMA: f32 = 0.25;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    o.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    o.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return o;
}

fn w_bilateral(c: vec4<f32>, s: vec4<f32>) -> f32 {
    let dw = exp(-abs(c.a - s.a) * FAR / DEPTH_SIGMA);
    let nw = max(dot(normalize(c.rgb), normalize(s.rgb)), 0.0);
    return dw * mix(0.2, 1.0, nw * nw);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let full = vec2<f32>(textureDimensions(t_normal_depth));
    let cnd = textureLoad(t_normal_depth, vec2<i32>(in.uv * full), 0);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let fpx = vec2<i32>(in.uv * full) + vec2<i32>(x, y);
            let snd = textureLoad(t_normal_depth, fpx, 0);
            let huv = (vec2<f32>(fpx) + 0.5) / full;
            let g = textureSample(t_gtao_half, s_gtao, huv);
            let w = w_bilateral(cnd, snd);
            acc += g * w;
            wsum += w;
        }
    }
    // Guard against zero weight on silhouettes (sky vs geometry) — normalize(0) → NaN.
    if (wsum < 1e-4) {
        let g = textureSample(t_gtao_half, s_gtao, in.uv);
        return vec4<f32>(normalize(g.rgb), g.a);
    }
    let out = acc / max(wsum, 1e-5);
    return vec4<f32>(normalize(out.rgb), out.a);
}

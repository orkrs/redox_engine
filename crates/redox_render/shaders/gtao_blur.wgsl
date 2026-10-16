@group(0) @binding(0) var t_gtao: texture_2d<f32>;
@group(0) @binding(1) var s_gtao: sampler;
@group(0) @binding(2) var t_normal_depth: texture_2d<f32>;

const FAR: f32 = 1000.0;
const DEPTH_SIGMA: f32 = 0.35;

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
    let ts = vec2<f32>(textureDimensions(t_gtao));
    let tsz = 1.0 / ts;
    let nds = vec2<f32>(textureDimensions(t_normal_depth));
    let cnd = textureLoad(t_normal_depth, vec2<i32>(in.uv * nds), 0);

    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let uv = in.uv + vec2<f32>(f32(x), f32(y)) * tsz;
            let snd = textureLoad(t_normal_depth, vec2<i32>(clamp(uv * nds, vec2<f32>(0.0), nds - 1.0)), 0);
            let g = textureSample(t_gtao, s_gtao, uv);
            let w = w_bilateral(cnd, snd) * exp(-0.15 * f32(x * x + y * y));
            acc += g * w;
            wsum += w;
        }
    }
    let out = acc / max(wsum, 1e-5);
    return vec4<f32>(normalize(out.rgb), out.a);
}

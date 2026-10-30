// Screen-space subsurface blur (MAT-05 step 3): one separable pass of the HDR colour, only on
// pixels the normal prepass marked as subsurface (`surface.b`), with weights per colour channel
// (red scatters furthest) and a depth test so the blur does not cross silhouettes.
// Runs twice per frame, horizontally then vertically. Mirrors `src/post/sss.rs`.
// v1 simplification: the whole colour of the pixel is blurred, not only its diffuse part.

struct SssUniform {
    direction       : vec2<f32>,
    screen          : vec2<f32>,
    radius_m        : f32,
    depth_tolerance : f32,
    proj_y          : f32,
    pad0            : f32,
    profile         : vec4<f32>,
};

const NORMAL_DEPTH_RANGE : f32 = 1000.0;
const TAPS : i32 = 8;
const MAX_RADIUS_PX : f32 = 24.0;

@group(0) @binding(0) var t_color   : texture_2d<f32>;
@group(0) @binding(1) var t_normal  : texture_2d<f32>;
@group(0) @binding(2) var t_surface : texture_2d<f32>;
@group(0) @binding(3) var s_linear  : sampler;
@group(0) @binding(4) var<uniform>   sss : SssUniform;

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

// Texel at a uv, read with textureLoad so the targets may differ in size.
fn load_px(tex: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(tex));
    let px = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(tex, px, 0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let centre = textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    let surface = load_px(t_surface, in.uv);
    if (surface.b < 0.5) {
        return centre;
    }
    let depth_c = load_px(t_normal, in.uv).a * NORMAL_DEPTH_RANGE;
    // World radius -> pixels at this depth.
    let radius_px = clamp(
        sss.radius_m * sss.screen.y * sss.proj_y * 0.5 / max(depth_c, 0.05),
        0.0,
        MAX_RADIUS_PX,
    );
    if (radius_px < 0.5) {
        return centre;
    }
    let texel = 1.0 / sss.screen;
    var acc = vec3<f32>(0.0);
    var wsum = vec3<f32>(0.0);
    for (var i = -TAPS; i <= TAPS; i = i + 1) {
        let t = f32(i) / f32(TAPS);
        let uv = in.uv + sss.direction * texel * (t * radius_px);
        let s = load_px(t_surface, uv);
        let d = load_px(t_normal, uv).a * NORMAL_DEPTH_RANGE;
        if (s.b < 0.5 || abs(d - depth_c) > sss.depth_tolerance) {
            continue;
        }
        let sigma = max(sss.profile.xyz, vec3<f32>(0.05));
        let w = exp(-(t * t) * 0.5 / (sigma * sigma));
        acc = acc + textureSampleLevel(t_color, s_linear, uv, 0.0).rgb * w;
        wsum = wsum + w;
    }
    let blurred = select(centre.rgb, acc / max(wsum, vec3<f32>(1.0e-5)), wsum.x > 0.0);
    return vec4<f32>(blurred, centre.a);
}

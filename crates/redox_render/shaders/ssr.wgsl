// Screen-space reflections. Mirrors roughness_weight, edge_fade, fresnel_weight and ssr_weight
// in src/post/ssr.rs; keep in sync.
// The normal target holds world normal (xyz) and view depth / NORMAL_DEPTH_RANGE (w).
// The material surface target holds roughness (r) and metallic (g).
// NORMAL_DEPTH_RANGE must equal FAR in shaders/normal.wgsl.

struct SsrUniform {
    view          : mat4x4<f32>,
    proj          : mat4x4<f32>,
    inv_proj      : mat4x4<f32>,
    screen        : vec2<f32>,
    max_roughness : f32,
    max_distance  : f32,
    thickness     : f32,
    intensity     : f32,
    steps         : u32,
    pad0          : u32,
    pad1          : u32,
    pad2          : u32,
    pad3          : u32,
    pad4          : u32,
};

const NORMAL_DEPTH_RANGE : f32 = 1000.0;
const MAX_WEIGHT : f32 = 0.9;
const EDGE_BAND : f32 = 0.1;
const BISECTION_STEPS : u32 = 5u;

@group(0) @binding(0) var t_color   : texture_2d<f32>;
@group(0) @binding(1) var t_normal  : texture_2d<f32>;
@group(0) @binding(2) var t_surface : texture_2d<f32>;
@group(0) @binding(3) var s_linear  : sampler;
@group(0) @binding(4) var<uniform>   ssr : SsrUniform;

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

fn roughness_weight(roughness: f32, max_roughness: f32) -> f32 {
    if (max_roughness <= 0.0) {
        return 0.0;
    }
    let t = clamp(1.0 - roughness / max_roughness, 0.0, 1.0);
    return t * t;
}

fn edge_fade(uv: vec2<f32>) -> f32 {
    let d = min(uv, vec2<f32>(1.0) - uv);
    let f = clamp(d / EDGE_BAND, vec2<f32>(0.0), vec2<f32>(1.0));
    return f.x * f.y;
}

fn fresnel_weight(n_dot_v: f32, metallic: f32) -> f32 {
    let f0 = mix(0.04, 1.0, metallic);
    let c = clamp(n_dot_v, 0.0, 1.0);
    return f0 + (1.0 - f0) * pow(1.0 - c, 5.0);
}

fn ssr_weight(roughness: f32, metallic: f32, n_dot_v: f32, hit_uv: vec2<f32>) -> f32 {
    let w = roughness_weight(roughness, ssr.max_roughness)
        * edge_fade(hit_uv)
        * fresnel_weight(n_dot_v, metallic)
        * ssr.intensity;
    return clamp(w, 0.0, MAX_WEIGHT);
}

// uv of a view-space point, or (-1, -1) when it is behind the eye.
fn project_uv(p: vec3<f32>) -> vec2<f32> {
    let c = ssr.proj * vec4<f32>(p, 1.0);
    if (c.w <= 1e-6) {
        return vec2<f32>(-1.0, -1.0);
    }
    let ndc = c.xy / c.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

fn in_screen(uv: vec2<f32>) -> bool {
    return all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
}

fn scene_depth_at(uv: vec2<f32>) -> f32 {
    return load_px(t_normal, uv).a * NORMAL_DEPTH_RANGE;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    let surface = load_px(t_surface, in.uv);
    let roughness = surface.r;
    let metallic = surface.g;
    if (roughness_weight(roughness, ssr.max_roughness) <= 0.0) {
        return color;
    }

    let nd = load_px(t_normal, in.uv);
    let scene_z = nd.a * NORMAL_DEPTH_RANGE;
    if (scene_z <= 0.0) {
        return color;
    }
    let n = normalize((ssr.view * vec4<f32>(normalize(nd.xyz), 0.0)).xyz);

    // View-space position of this pixel: the far-plane point in its direction, scaled so
    // that its depth along -z equals the stored view depth.
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let far_pt = ssr.inv_proj * vec4<f32>(ndc, 1.0, 1.0);
    let dir = far_pt.xyz / far_pt.w;
    let p = dir * (scene_z / -dir.z);
    let v = normalize(p);
    let r = reflect(v, n);
    // A ray heading back towards the eye sees nothing on screen.
    if (r.z >= 0.0) {
        return color;
    }
    let n_dot_v = clamp(dot(n, -v), 0.0, 1.0);

    let steps = max(ssr.steps, 1u);
    let step_len = ssr.max_distance / f32(steps);
    var lo = 0.0;
    var hi = 0.0;
    var hit = false;
    for (var i = 1u; i <= steps; i = i + 1u) {
        let t = f32(i) * step_len;
        let q = p + r * t;
        let uv = project_uv(q);
        if (!in_screen(uv)) {
            break;
        }
        let gap = -q.z - scene_depth_at(uv);
        if (gap > 0.0 && gap < ssr.thickness) {
            hit = true;
            hi = t;
            break;
        }
        lo = t;
    }
    if (!hit) {
        return color;
    }

    for (var k = 0u; k < BISECTION_STEPS; k = k + 1u) {
        let mid = 0.5 * (lo + hi);
        let q = p + r * mid;
        let gap = -q.z - scene_depth_at(project_uv(q));
        if (gap > 0.0) {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    let hit_uv = project_uv(p + r * hi);
    let hit_color = textureSampleLevel(t_color, s_linear, hit_uv, 0.0);
    let w = ssr_weight(roughness, metallic, n_dot_v, hit_uv);
    return mix(color, hit_color, w);
}

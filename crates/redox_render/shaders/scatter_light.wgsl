//! ScatterLight — RedOx stochastic per-pixel point-light path (K=8 samples).
//!
//! Reads point lights from a storage buffer and accumulates K randomly
//! selected lights per pixel. Reservoir sampling / shadow rays — follow-up.

struct PointLight {
    position: vec4<f32>,
    color: vec4<f32>,
    shadow_matrices: array<mat4x4<f32>, 6>,
    intensity: f32,
    radius: f32,
    shadow_tile_base: u32,
    _padding: f32,
};

struct ScatterLightUniform {
    num_lights: u32,
    sample_count: u32,
    frame_index: u32,
    _pad: u32,
    resolution: vec2<f32>,
    _pad2: vec2<f32>,
    inv_view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var t_hdr: texture_2d<f32>;
@group(0) @binding(1) var t_depth: texture_depth_2d;
@group(0) @binding(2) var s_linear: sampler;
@group(0) @binding(3) var<storage, read> point_lights: array<PointLight>;
@group(0) @binding(4) var<uniform> sl_u: ScatterLightUniform;

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

fn hash_u32(x: u32) -> u32 {
    var v = x;
    v = v ^ (v >> 16u);
    v = v * 0x7feb352du;
    v = v ^ (v >> 15u);
    v = v * 0x846ca68bu;
    v = v ^ (v >> 16u);
    return v;
}

/// World-space position of the pixel at integer coordinate `p` (depth in [0, 1]).
fn world_at(p: vec2<i32>) -> vec3<f32> {
    let res = vec2<i32>(sl_u.resolution);
    let q = clamp(p, vec2<i32>(0), res - vec2<i32>(1));
    let depth = textureLoad(t_depth, q, 0);
    let uv = (vec2<f32>(q) + vec2<f32>(0.5)) / sl_u.resolution;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let w = sl_u.inv_view_proj * ndc;
    return w.xyz / w.w;
}

fn evaluate_point_light(light: PointLight, world_pos: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let L = light.position.xyz - world_pos;
    let dist = length(L);
    if (dist > light.radius || dist < 1e-4) {
        return vec3<f32>(0.0);
    }
    let atten = pow(max(1.0 - dist / light.radius, 0.0), 2.0);
    let NdotL = max(dot(L / dist, normal), 0.0);
    return light.color.xyz * light.intensity * atten * NdotL;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = textureSample(t_hdr, s_linear, in.uv);
    let n = sl_u.num_lights;
    if (n == 0u) {
        return base;
    }

    let px = u32(in.uv.x * sl_u.resolution.x);
    let py = u32(in.uv.y * sl_u.resolution.y);
    let K = sl_u.sample_count;

    // Reconstruct world position and a geometric normal from the depth buffer.
    let pix = vec2<i32>(i32(px), i32(py));
    let max_pix = vec2<i32>(sl_u.resolution) - vec2<i32>(1);
    if (textureLoad(t_depth, clamp(pix, vec2<i32>(0), max_pix), 0) >= 1.0) {
        return base; // sky: nothing to light
    }
    let world_pos = world_at(pix);
    let dpx = world_at(pix + vec2<i32>(1, 0)) - world_pos;
    let dpy = world_at(pix + vec2<i32>(0, 1)) - world_pos;
    let normal = normalize(cross(dpy, dpx));

    var accum = vec3<f32>(0.0);
    for (var k: u32 = 0u; k < K; k = k + 1u) {
        let seed = hash_u32(px + py * 73856093u + k * 19349663u + sl_u.frame_index);
        let idx = seed % n;
        let light = point_lights[idx];
        accum = accum + evaluate_point_light(light, world_pos, normal);
    }
    accum = accum / f32(K);

    return vec4<f32>(base.rgb + accum, base.a);
}

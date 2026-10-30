// Probe atlas update (GI-01 step 4): irradiance and visibility per probe tile, with a one-texel
// border. One workgroup per tile: workgroup_id.xy addresses the tile, probe = y * probes_per_row + x.
//
// Inputs are the ray records written by gi_trace.wgsl (ray_data[probe * rays + ray]); the ray
// directions are recomputed here with the same rotation and Fibonacci mirror as gi_trace.wgsl.
// The atlases are ping-ponged: the old values are read from the `*_old` textures and the new
// ones are written to the `*_new` textures, so no texture is read and written in one dispatch.
// Border texels are not copied from the new texture; each one recomputes the value of its
// octahedral source texel (border_source), which keeps this file to a single dispatch.
//
// CPU mirrors: gi::oct (oct_encode, oct_decode, border_source, fibonacci_sphere), gi::atlas.
// Storage: irradiance is kept as acc^(1/5) (perceptual), visibility as (mean t, mean t^2).

struct UpdateParams {
    layout: vec4<u32>,  // probes per row (x), probe count (y), rays per probe (z), frame index (w)
    blend: vec4<f32>,   // hysteresis: weight of the old value (x); visibility distance clamp (y)
};

@group(0) @binding(0) var<uniform> params: UpdateParams;
@group(0) @binding(1) var t_irr_old: texture_2d<f32>;
@group(0) @binding(2) var t_vis_old: texture_2d<f32>;
@group(0) @binding(3) var t_irr_new: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var t_vis_new: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var<storage, read> ray_data: array<vec4<f32>>;

const PI: f32 = 3.14159265;
const GOLDEN_ANGLE: f32 = 2.39996323;
const IRR_INNER: u32 = 8u;
const IRR_STRIDE: u32 = 10u;
const VIS_INNER: u32 = 16u;
const VIS_STRIDE: u32 = 18u;
const PERCEPTUAL_POWER: f32 = 0.2;

fn sign_nz(x: f32) -> f32 {
    return select(-1.0, 1.0, x >= 0.0);
}

// Mirror of gi::oct::oct_encode.
fn oct_encode(dir: vec3<f32>) -> vec2<f32> {
    let n = dir / max(abs(dir.x) + abs(dir.y) + abs(dir.z), 1e-6);
    var u = n.x;
    var v = n.z;
    if (n.y < 0.0) {
        let ou = u;
        u = (1.0 - abs(v)) * sign_nz(ou);
        v = (1.0 - abs(ou)) * sign_nz(v);
    }
    return clamp(vec2<f32>(u, v) * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
}

// Mirror of gi::oct::oct_decode. Accepts uv outside [0, 1]: the fold handles it.
fn oct_decode(uv: vec2<f32>) -> vec3<f32> {
    var f = vec3<f32>(uv.x * 2.0 - 1.0, 0.0, uv.y * 2.0 - 1.0);
    f.y = 1.0 - abs(f.x) - abs(f.z);
    if (f.y < 0.0) {
        let ox = f.x;
        f.x = (1.0 - abs(f.z)) * sign_nz(ox);
        f.z = (1.0 - abs(ox)) * sign_nz(f.z);
    }
    return f / max(length(f), 1e-12);
}

// Mirror of gi::oct::border_source: tile coordinate of a border texel (ring at 0 and inner + 1)
// to the inner tile coordinate (1..=inner) it copies.
fn border_source(b: vec2<u32>, inner: u32) -> vec2<u32> {
    let n = f32(inner);
    let uv = (vec2<f32>(b) - 0.5) / n;
    let e = oct_encode(oct_decode(uv));
    let i = clamp(floor(e * n), vec2<f32>(0.0), vec2<f32>(n - 1.0));
    return vec2<u32>(i) + vec2<u32>(1u);
}

fn hash_u32(x: u32) -> u32 {
    let s = x * 747796405u + 2891336453u;
    let word = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash_unit(x: u32) -> f32 {
    return f32(hash_u32(x)) * (1.0 / 4294967296.0);
}

// Mirror of gi_trace.wgsl random_rotation (Shoemake).
fn random_rotation(seed: u32) -> vec4<f32> {
    let u0 = hash_unit(seed);
    let u1 = hash_unit(seed ^ 0x9E3779B9u);
    let u2 = hash_unit(seed ^ 0x85EBCA6Bu);
    let r1 = sqrt(1.0 - u0);
    let r2 = sqrt(u0);
    let a = 2.0 * PI * u1;
    let b = 2.0 * PI * u2;
    return vec4<f32>(r1 * sin(a), r1 * cos(a), r2 * sin(b), r2 * cos(b));
}

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// Mirror of gi_trace.wgsl fib_dir.
fn fib_dir(i: u32, n: u32) -> vec3<f32> {
    let fi = f32(i) + 0.5;
    let z = 1.0 - 2.0 * fi / f32(n);
    let r = sqrt(max(0.0, 1.0 - z * z));
    let phi = fi * GOLDEN_ANGLE;
    return vec3<f32>(r * cos(phi), r * sin(phi), z);
}

// cos^50 by squaring (cos >= 0), avoiding pow(0, x).
fn cos_pow50(c: f32) -> f32 {
    let c2 = c * c;
    let c4 = c2 * c2;
    let c8 = c4 * c4;
    let c16 = c8 * c8;
    let c32 = c16 * c16;
    return c32 * c16 * c2;
}

fn tile_origin(workgroup: vec2<u32>, stride: u32) -> vec2<u32> {
    return workgroup * stride;
}

// Irradiance at inner texel `inner` (0..IRR_INNER) of `probe`, mixed with the old texel.
// acc = (4 pi / rays) * sum L cos, which is pi for unit radiance over the sphere.
fn irradiance_value(probe: u32, inner: vec2<u32>, origin: vec2<u32>, q: vec4<f32>) -> vec4<f32> {
    let rays = params.layout.z;
    let n = oct_decode((vec2<f32>(inner) + 0.5) / f32(IRR_INNER));
    var acc = vec3<f32>(0.0);
    for (var r = 0u; r < rays; r = r + 1u) {
        let rd = rotate(q, fib_dir(r, rays));
        let radiance = ray_data[probe * rays + r].rgb;
        acc = acc + radiance * max(dot(n, rd), 0.0);
    }
    acc = acc * (4.0 * PI / f32(rays));
    let target_value = pow(max(acc, vec3<f32>(1e-8)), vec3<f32>(PERCEPTUAL_POWER));
    let old = textureLoad(t_irr_old, vec2<i32>(origin + vec2<u32>(1u) + inner), 0).rgb;
    let mixed = mix(target_value, old, params.blend.x);
    return vec4<f32>(mixed, 1.0);
}

// Mean and mean-square distance at visibility texel `inner` (0..VIS_INNER), mixed with the old.
fn visibility_value(probe: u32, inner: vec2<u32>, origin: vec2<u32>, q: vec4<f32>) -> vec4<f32> {
    let rays = params.layout.z;
    let n = oct_decode((vec2<f32>(inner) + 0.5) / f32(VIS_INNER));
    var weight_sum = 0.0;
    var acc = vec2<f32>(0.0);
    for (var r = 0u; r < rays; r = r + 1u) {
        let rd = rotate(q, fib_dir(r, rays));
        let w = cos_pow50(max(dot(n, rd), 0.0));
        let t = min(ray_data[probe * rays + r].w, params.blend.y);
        acc = acc + vec2<f32>(t, t * t) * w;
        weight_sum = weight_sum + w;
    }
    let mean = acc / max(weight_sum, 1e-6);
    let old = textureLoad(t_vis_old, vec2<i32>(origin + vec2<u32>(1u) + inner), 0).rg;
    let mixed = mix(mean, old, params.blend.x);
    return vec4<f32>(mixed, 0.0, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn update_irradiance(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(local_invocation_index) lindex: u32,
) {
    let ppr = params.layout.x;
    let probe = wg.y * ppr + wg.x;
    if (probe >= params.layout.y) {
        return;
    }
    let q = random_rotation(hash_u32(params.layout.w));
    let origin = tile_origin(wg.xy, IRR_STRIDE);
    textureStore(
        t_irr_new,
        vec2<i32>(origin + vec2<u32>(1u) + lid.xy),
        irradiance_value(probe, lid.xy, origin, q),
    );
    // Border ring: every texel of the tile is visited by one of the 64 threads.
    for (var k = lindex; k < IRR_STRIDE * IRR_STRIDE; k = k + 64u) {
        let b = vec2<u32>(k % IRR_STRIDE, k / IRR_STRIDE);
        if (b.x == 0u || b.y == 0u || b.x == IRR_STRIDE - 1u || b.y == IRR_STRIDE - 1u) {
            let src = border_source(b, IRR_INNER) - vec2<u32>(1u);
            textureStore(
                t_irr_new,
                vec2<i32>(origin + b),
                irradiance_value(probe, src, origin, q),
            );
        }
    }
}

@compute @workgroup_size(16, 16, 1)
fn update_visibility(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
    @builtin(local_invocation_index) lindex: u32,
) {
    let ppr = params.layout.x;
    let probe = wg.y * ppr + wg.x;
    if (probe >= params.layout.y) {
        return;
    }
    let q = random_rotation(hash_u32(params.layout.w));
    let origin = tile_origin(wg.xy, VIS_STRIDE);
    textureStore(
        t_vis_new,
        vec2<i32>(origin + vec2<u32>(1u) + lid.xy),
        visibility_value(probe, lid.xy, origin, q),
    );
    for (var k = lindex; k < VIS_STRIDE * VIS_STRIDE; k = k + 256u) {
        let b = vec2<u32>(k % VIS_STRIDE, k / VIS_STRIDE);
        if (b.x == 0u || b.y == 0u || b.x == VIS_STRIDE - 1u || b.y == VIS_STRIDE - 1u) {
            let src = border_source(b, VIS_INNER) - vec2<u32>(1u);
            textureStore(
                t_vis_new,
                vec2<i32>(origin + b),
                visibility_value(probe, src, origin, q),
            );
        }
    }
}

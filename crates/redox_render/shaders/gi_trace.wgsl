// Probe ray trace against the global SDF (GI-01 step 3), with surface cards (GI-02 step 4).
// gi_common.wgsl is concatenated in front of this file (shader/mod.rs): PI, the GiGridGpu mirror
// and the previous-frame irradiance helpers come from there.
//
// One invocation traces one probe ray. Directions are a Fibonacci sphere rotated by a random
// quaternion per frame (CPU mirror: gi::oct::fibonacci_sphere). A hit is shaded with emission,
// a sun term and the bounce term (last frame's probe irradiance at the hit point); a miss returns
// the sky prefilter. Output:
//   ray_data[probe * rays + ray] = (radiance.rgb, hit distance or MAX_TRACE_DISTANCE).

struct TraceParams {
    grid_origin: vec4<f32>,  // probe lattice origin (xyz), probe spacing (w)
    grid_dims: vec4<u32>,    // probe counts per axis (xyz), rays per probe (w)
    sun_dir: vec4<f32>,      // unit direction towards the sun (xyz)
    sun_rad: vec4<f32>,      // sun radiance (xyz)
    frame: vec4<u32>,        // frame index (x), probe count of this dispatch (y), first atlas probe (z)
    scroll: vec4<i32>,       // lowest world cell of the cascade window (xyz)
};

struct SdfInfo {
    origin_voxel: vec4<f32>, // volume minimum corner (xyz), voxel size (w)
    dims: vec4<u32>,         // voxel counts (xyz)
};

// x: 1 when cards are read, y: number of table entries (object ids below y have cards).
struct CardTraceParams {
    enabled: vec4<u32>,
}

struct March {
    t: f32,
    hit: bool,
};

@group(0) @binding(0) var<uniform> params: TraceParams;
@group(0) @binding(1) var t_distance: texture_3d<f32>;
@group(0) @binding(2) var t_albedo: texture_3d<f32>;
@group(0) @binding(3) var s_linear: sampler;
@group(0) @binding(4) var t_sky: texture_cube<f32>;
@group(0) @binding(5) var<storage, read_write> ray_data: array<vec4<f32>>;
@group(0) @binding(6) var<uniform> sdf: SdfInfo;
// Grid and irradiance atlas of the previous frame: the atlas is what the bounce term reads.
@group(0) @binding(7) var<uniform> prev_grid: GiGridGpu;
@group(0) @binding(8) var t_prev_irradiance: texture_2d<f32>;
// Relocated probe positions (offset from the lattice point), indexed by atlas index.
@group(0) @binding(9) var<storage, read> probe_offset: array<vec4<f32>>;
// Surface cards (GI-02 step 4): radiance and normal/depth atlases, the object table and the
// object id volume. Bindings 10-14 come after the probe bindings.
@group(0) @binding(10) var t_card_radiance: texture_2d<f32>;
@group(0) @binding(11) var t_card_nd: texture_2d<f32>;
@group(0) @binding(12) var<storage, read> cards: array<CardObjectGpu>;
@group(0) @binding(13) var t_object_id: texture_3d<u32>;
@group(0) @binding(14) var<uniform> card_params: CardTraceParams;

const GOLDEN_ANGLE: f32 = 2.39996323;
const MAX_TRACE_DISTANCE: f32 = 40.0;
const MAX_STEPS: u32 = 48u;
const EMISSION_SCALE: f32 = 16.0;
const SKY_LOD: f32 = 4.0;

fn sdf_uvw(p: vec3<f32>) -> vec3<f32> {
    let extent = vec3<f32>(sdf.dims.xyz) * sdf.origin_voxel.w;
    return (p - sdf.origin_voxel.xyz) / extent;
}

fn sdf_distance(p: vec3<f32>) -> f32 {
    return textureSampleLevel(t_distance, s_linear, sdf_uvw(p), 0.0).r;
}

// Central-difference gradient of the distance field, normalised.
fn sdf_normal(p: vec3<f32>) -> vec3<f32> {
    let h = sdf.origin_voxel.w;
    let g = vec3<f32>(
        sdf_distance(p + vec3<f32>(h, 0.0, 0.0)) - sdf_distance(p - vec3<f32>(h, 0.0, 0.0)),
        sdf_distance(p + vec3<f32>(0.0, h, 0.0)) - sdf_distance(p - vec3<f32>(0.0, h, 0.0)),
        sdf_distance(p + vec3<f32>(0.0, 0.0, h)) - sdf_distance(p - vec3<f32>(0.0, 0.0, h)),
    );
    let len = length(g);
    if (len < 1e-8) {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return g / len;
}

// Sphere tracing over the distance field; a hit is a sample closer than half a voxel.
fn march(origin: vec3<f32>, dir: vec3<f32>) -> March {
    let eps = 0.5 * sdf.origin_voxel.w;
    var t = 0.0;
    for (var i = 0u; i < MAX_STEPS; i = i + 1u) {
        if (t > MAX_TRACE_DISTANCE) {
            break;
        }
        let d = sdf_distance(origin + dir * t);
        if (d < eps) {
            return March(t, true);
        }
        t = t + d;
    }
    return March(MAX_TRACE_DISTANCE, false);
}

// Integer hash (PCG-style), identical to the CPU reference generator's constants.
fn hash_u32(x: u32) -> u32 {
    let s = x * 747796405u + 2891336453u;
    let word = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash_unit(x: u32) -> f32 {
    return f32(hash_u32(x)) * (1.0 / 4294967296.0);
}

// Uniform random unit quaternion (xyzw) from a seed (Shoemake's method).
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

// The i-th of n Fibonacci-sphere directions (CPU mirror: gi::oct::fibonacci_sphere).
fn fib_dir(i: u32, n: u32) -> vec3<f32> {
    let fi = f32(i) + 0.5;
    let z = 1.0 - 2.0 * fi / f32(n);
    let r = sqrt(max(0.0, 1.0 - z * z));
    let phi = fi * GOLDEN_ANGLE;
    return vec3<f32>(r * cos(phi), r * sin(phi), z);
}

// Card depth tolerance as a fraction of the object AABB (a card from behind a wall is rejected).
const CARD_DEPTH_TOLERANCE: f32 = 0.06;
// Faces blended at a hit point (the three with the largest face weight).
const CARD_FACES_BLENDED: u32 = 3u;

// Object id of the voxel nearest to `p` (0xFFFF = none).
fn object_at(p: vec3<f32>) -> u32 {
    let v = clamp(
        vec3<i32>(floor((p - sdf.origin_voxel.xyz) / sdf.origin_voxel.w)),
        vec3<i32>(0),
        vec3<i32>(sdf.dims.xyz) - vec3<i32>(1),
    );
    return textureLoad(t_object_id, v, 0).r;
}

// Radiance seen on one face of a card at `p`: rgb, and w = 1 when the card is accepted (it has
// texels here and its depth matches the point within CARD_DEPTH_TOLERANCE).
fn card_face_sample(card: CardObjectGpu, face: u32, p: vec3<f32>) -> vec4<f32> {
    let rect = card.rects[face];
    if (rect.z == 0u || rect.w == 0u) {
        return vec4<f32>(0.0);
    }
    let proj = card_project(face, card.aabb_min.xyz, card.aabb_max.xyz, p);
    if (any(proj.xy < vec2<f32>(0.0)) || any(proj.xy > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let size = vec2<f32>(f32(rect.z), f32(rect.w));
    let local = min(vec2<u32>(proj.xy * size), vec2<u32>(rect.z - 1u, rect.w - 1u));
    let texel = vec2<i32>(i32(rect.x + local.x), i32(rect.y + local.y));
    let nd = textureLoad(t_card_nd, texel, 0);
    let code = u32(round(nd.a * 255.0));
    if (code >= 255u) {
        return vec4<f32>(0.0);
    }
    let card_depth = f32(code) / 254.0;
    if (abs(card_depth - proj.z) >= CARD_DEPTH_TOLERANCE) {
        return vec4<f32>(0.0);
    }
    // Bilinear radiance, sampled inside the rectangle so the footprint stays on the card.
    let atlas = vec2<f32>(textureDimensions(t_card_radiance));
    let inside = clamp(proj.xy * size, vec2<f32>(0.5), size - vec2<f32>(0.5));
    let uv = (vec2<f32>(rect.xy) + inside) / atlas;
    return vec4<f32>(textureSampleLevel(t_card_radiance, s_linear, uv, 0.0).rgb, 1.0);
}

// Card radiance at a hit point: the mean over the accepted faces among the CARD_FACES_BLENDED
// with the largest face weights. w = 0 when no face is accepted (the probe path is used).
fn card_radiance(p: vec3<f32>, n: vec3<f32>, object: u32) -> vec4<f32> {
    if (card_params.enabled.x == 0u || object >= card_params.enabled.y) {
        return vec4<f32>(0.0);
    }
    let card = cards[object];
    var weights = card_face_weights(n);
    var taken = 0u;
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var k = 0u; k < CARD_FACES_BLENDED; k = k + 1u) {
        var best = 6u;
        var best_w = 0.0;
        for (var f = 0u; f < 6u; f = f + 1u) {
            if (((taken >> f) & 1u) == 0u && weights[f] > best_w) {
                best = f;
                best_w = weights[f];
            }
        }
        if (best == 6u) {
            break;
        }
        taken = taken | (1u << best);
        let s = card_face_sample(card, best, p);
        if (s.w > 0.5) {
            acc = acc + s.rgb * best_w;
            wsum = wsum + best_w;
        }
    }
    if (wsum <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(acc / wsum, 1.0);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let rays = params.grid_dims.w;
    let index = gid.x;
    if (index >= params.frame.y * rays) {
        return;
    }
    let probe = index / rays;
    let ray = index % rays;
    let dims = params.grid_dims.xyz;
    // Ring slot of the probe, then the world cell that slot holds in the current window:
    // cell = scroll + ((slot - scroll) mod dims), the inverse of ProbeCascade::storage_index.
    let slot = vec3<i32>(
        i32(probe % dims.x),
        i32((probe / dims.x) % dims.y),
        i32(probe / (dims.x * dims.y)),
    );
    let d = vec3<i32>(dims);
    let rel = slot - params.scroll.xyz;
    let cell = params.scroll.xyz + ((rel % d) + d) % d;
    let origin = params.grid_origin.xyz + vec3<f32>(cell) * params.grid_origin.w;
    let q = random_rotation(hash_u32(params.frame.x));
    let dir = rotate(q, fib_dir(ray, rays));

    let m = march(origin, dir);
    var radiance = vec3<f32>(0.0);
    var hit_t = MAX_TRACE_DISTANCE;
    if (m.hit) {
        let p = origin + dir * m.t;
        let n = sdf_normal(p);
        let carded = card_radiance(p, n, object_at(p));
        if (carded.w > 0.5) {
            // Lit by the card light pass (sun, points, previous-frame irradiance).
            radiance = carded.rgb;
        } else {
            let surface = textureSampleLevel(t_albedo, s_linear, sdf_uvw(p), 0.0);
            radiance = surface.rgb * surface.a * EMISSION_SCALE;
            // Direct sun, with a shadow ray that starts one voxel off the surface.
            let sun = params.sun_dir.xyz;
            let ndl = max(dot(n, sun), 0.0);
            if (ndl > 0.0 && !march(p + n * sdf.origin_voxel.w, sun).hit) {
                radiance = radiance + params.sun_rad.xyz * ndl * surface.rgb / PI;
            }
            // Bounce: last frame's irradiance at the hit point. Albedo over pi (Lambert BRDF), so the
            // loop gain per bounce is the albedo, not albedo * pi.
            radiance = radiance + surface.rgb * prev_irradiance(p, n) / PI;
        }
        hit_t = m.t;
    } else {
        radiance = textureSampleLevel(t_sky, s_linear, dir, SKY_LOD).rgb;
    }
    ray_data[(params.frame.z + probe) * rays + ray] = vec4<f32>(radiance, hit_t);
}

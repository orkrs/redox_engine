// Probe ray trace against the global SDF (GI-01 step 3).
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

struct March {
    t: f32,
    hit: bool,
};

// Mirror of pbr.wgsl GiCascadeGpu / GiGridGpu (src/gi/grid_gpu.rs), for the previous frame.
struct GiCascadeGpu {
    origin: vec4<f32>,   // xyz origin, w probe spacing
    dims: vec4<u32>,     // xyz probe counts, w atlas index of the first probe
    scroll: vec4<i32>,   // xyz lowest world cell of the window
}
struct GiGridGpu {
    cascades: array<GiCascadeGpu, 2>,
    probes_per_row: u32,
    enabled: u32,
    cascade_count: u32,
    _pad: u32,
}

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

const PI: f32 = 3.14159265;
const GOLDEN_ANGLE: f32 = 2.39996323;
const MAX_TRACE_DISTANCE: f32 = 40.0;
const MAX_STEPS: u32 = 48u;
const EMISSION_SCALE: f32 = 16.0;
const SKY_LOD: f32 = 4.0;
// Irradiance tile geometry (CPU: gi::atlas::{IRRADIANCE_INNER, TILE_BORDER}).
const GI_INNER_IRR: f32 = 8.0;
const GI_STRIDE_IRR: u32 = 10u;
// Shading point pushed off the surface along the normal, in cascade spacings.
const BOUNCE_NORMAL_BIAS: f32 = 0.2;

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

// Mirror of pbr.wgsl oct_encode.
fn oct_encode(n: vec3<f32>) -> vec2<f32> {
    var v = n / max(abs(n.x) + abs(n.y) + abs(n.z), 1.0e-6);
    if (v.y < 0.0) {
        let ox = v.x;
        v.x = (1.0 - abs(v.z)) * select(-1.0, 1.0, ox >= 0.0);
        v.z = (1.0 - abs(ox)) * select(-1.0, 1.0, v.z >= 0.0);
    }
    return clamp(v.xz * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
}

// Previous frame's irradiance E in the direction `dir` of atlas probe `probe`, bilinear inside
// its tile (the stored value is E^(1/5); see gi::atlas).
fn prev_tile_irradiance(probe: u32, dir: vec3<f32>) -> vec3<f32> {
    let ppr = prev_grid.probes_per_row;
    let tile = vec2<f32>(f32(probe % ppr), f32(probe / ppr)) * f32(GI_STRIDE_IRR);
    let p = tile + 1.0 + oct_encode(dir) * GI_INNER_IRR - 0.5;
    let i0 = floor(p);
    let f = p - i0;
    let c = vec2<i32>(i0);
    let a = textureLoad(t_prev_irradiance, c, 0).rgb;
    let b = textureLoad(t_prev_irradiance, c + vec2<i32>(1, 0), 0).rgb;
    let d = textureLoad(t_prev_irradiance, c + vec2<i32>(0, 1), 0).rgb;
    let e = textureLoad(t_prev_irradiance, c + vec2<i32>(1, 1), 0).rgb;
    let stored = mix(mix(a, b, f.x), mix(d, e, f.x), f.y);
    return pow(max(stored, vec3<f32>(0.0)), vec3<f32>(5.0));
}

// Trilinear blend of the eight probes around `p` in one cascade, without the visibility test
// (the bounce term is cheap by design). Returns (weighted E sum, total weight).
fn prev_sample_cascade(casc: GiCascadeGpu, p: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let spacing = casc.origin.w;
    let biased = p + n * (BOUNCE_NORMAL_BIAS * spacing);
    let g = (biased - casc.origin.xyz) / spacing;
    let base = floor(g);
    let frac = g - base;
    let base_cell = vec3<i32>(base);
    let scroll = casc.scroll.xyz;
    let dims = vec3<i32>(casc.dims.xyz);
    var accum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dz = 0; dz < 2; dz = dz + 1) {
        for (var dy = 0; dy < 2; dy = dy + 1) {
            for (var dx = 0; dx < 2; dx = dx + 1) {
                let cell = base_cell + vec3<i32>(dx, dy, dz);
                let rel = cell - scroll;
                if (any(rel < vec3<i32>(0)) || any(rel >= dims)) {
                    continue;
                }
                let slot = ((cell % dims) + dims) % dims;
                let su = vec3<u32>(slot);
                let index = casc.dims.w + su.x + su.y * casc.dims.x + su.z * casc.dims.x * casc.dims.y;
                let probe_pos = casc.origin.xyz + vec3<f32>(cell) * spacing + probe_offset[index].xyz;
                let wx = select(frac.x, 1.0 - frac.x, dx == 0);
                let wy = select(frac.y, 1.0 - frac.y, dy == 0);
                let wz = select(frac.z, 1.0 - frac.z, dz == 0);
                // Backface term as in pbr.wgsl: probes behind the surface count less, not zero.
                let to_probe = probe_pos - p;
                let dir_to_probe = to_probe / max(length(to_probe), 1.0e-5);
                let facing = (dot(dir_to_probe, n) + 1.0) * 0.5;
                let w = wx * wy * wz * (facing * facing + 0.2);
                accum = accum + prev_tile_irradiance(index, n) * w;
                wsum = wsum + w;
            }
        }
    }
    return vec4<f32>(accum, wsum);
}

// True when `p` lies inside the world window of cascade `casc`.
fn prev_cascade_contains(casc: GiCascadeGpu, p: vec3<f32>) -> bool {
    let lo = casc.origin.xyz + vec3<f32>(casc.scroll.xyz) * casc.origin.w;
    let hi = lo + vec3<f32>(casc.dims.xyz) * casc.origin.w;
    return all(p >= lo) && all(p <= hi);
}

// Irradiance E of the previous frame at a hit point: the fine cascade when the point is in its
// window, else the coarse one. Zero when GI is off or the point is outside both.
fn prev_irradiance(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if (prev_grid.enabled == 0u) {
        return vec3<f32>(0.0);
    }
    let fine = prev_grid.cascades[0];
    if (prev_cascade_contains(fine, p)) {
        let s = prev_sample_cascade(fine, p, n);
        if (s.w > 1.0e-5) {
            return s.rgb / s.w;
        }
    }
    let coarse = prev_grid.cascades[1];
    if (prev_grid.cascade_count > 1u && prev_cascade_contains(coarse, p)) {
        let s = prev_sample_cascade(coarse, p, n);
        if (s.w > 1.0e-5) {
            return s.rgb / s.w;
        }
    }
    return vec3<f32>(0.0);
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
        hit_t = m.t;
    } else {
        radiance = textureSampleLevel(t_sky, s_linear, dir, SKY_LOD).rgb;
    }
    ray_data[(params.frame.z + probe) * rays + ray] = vec4<f32>(radiance, hit_t);
}

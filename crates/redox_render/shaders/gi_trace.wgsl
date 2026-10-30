// Probe ray trace against the global SDF (GI-01 step 3).
//
// One invocation traces one probe ray. Directions are a Fibonacci sphere rotated by a random
// quaternion per frame (CPU mirror: gi::oct::fibonacci_sphere). A hit is shaded with emission
// and a sun term; a miss returns the sky prefilter. Output:
//   ray_data[probe * rays + ray] = (radiance.rgb, hit distance or MAX_TRACE_DISTANCE).

struct TraceParams {
    grid_origin: vec4<f32>,  // probe lattice origin (xyz), probe spacing (w)
    grid_dims: vec4<u32>,    // probe counts per axis (xyz), rays per probe (w)
    sun_dir: vec4<f32>,      // unit direction towards the sun (xyz)
    sun_rad: vec4<f32>,      // sun radiance (xyz)
    frame: vec4<u32>,        // frame index (x), probe count (y)
};

struct SdfInfo {
    origin_voxel: vec4<f32>, // volume minimum corner (xyz), voxel size (w)
    dims: vec4<u32>,         // voxel counts (xyz)
};

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

const PI: f32 = 3.14159265;
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
    let cell = vec3<f32>(
        f32(probe % dims.x),
        f32((probe / dims.x) % dims.y),
        f32(probe / (dims.x * dims.y)),
    );
    let origin = params.grid_origin.xyz + cell * params.grid_origin.w;
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
        hit_t = m.t;
    } else {
        radiance = textureSampleLevel(t_sky, s_linear, dir, SKY_LOD).rgb;
    }
    ray_data[index] = vec4<f32>(radiance, hit_t);
}

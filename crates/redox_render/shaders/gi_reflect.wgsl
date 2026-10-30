// Rough reflections from the GI cache (GI-04). gi_common.wgsl is concatenated in front: PI, the
// GiGridGpu mirror and `prev_irradiance` come from there.
//
// One invocation per pixel: a reflected ray (spread by the roughness) is traced through the global
// SDF; at the hit the radiance is emission plus albedo * the probe irradiance of the previous frame
// / pi, the same shading the probe rays use. Output `(radiance.rgb, blend weight)`; weight 0 means
// no hit or not a reflective pixel. The post chain blends it over the colour before SSR (which
// overrides it where it finds a hit).
// The normal target holds world normal (xyz) and view depth / NORMAL_DEPTH_RANGE (w); the
// material surface target holds roughness (r) and metallic (g).

struct ReflectParams {
    inv_proj       : mat4x4<f32>,
    inv_view       : mat4x4<f32>,
    screen         : vec2<f32>,
    min_roughness  : f32,
    max_roughness  : f32,
    max_distance   : f32,
    intensity      : f32,
    frame          : u32,
    pad0           : u32,
};

struct SdfInfo {
    origin_voxel: vec4<f32>, // volume minimum corner (xyz), voxel size (w)
    dims: vec4<u32>,         // voxel counts (xyz)
};

@group(0) @binding(0) var<uniform> params: ReflectParams;
@group(0) @binding(1) var t_normal: texture_2d<f32>;
@group(0) @binding(2) var t_surface: texture_2d<f32>;
@group(0) @binding(3) var t_distance: texture_3d<f32>;
@group(0) @binding(4) var t_albedo: texture_3d<f32>;
@group(0) @binding(5) var s_linear: sampler;
@group(0) @binding(6) var<uniform> sdf: SdfInfo;
@group(0) @binding(7) var<uniform> prev_grid: GiGridGpu;
@group(0) @binding(8) var t_prev_irradiance: texture_2d<f32>;
@group(0) @binding(9) var<storage, read> probe_offset: array<vec4<f32>>;
@group(0) @binding(10) var out_reflect: texture_storage_2d<rgba16float, write>;

const NORMAL_DEPTH_RANGE: f32 = 1000.0;
const MAX_STEPS: u32 = 64u;
const EMISSION_SCALE: f32 = 16.0;
const MAX_WEIGHT: f32 = 0.9;

fn sdf_uvw(p: vec3<f32>) -> vec3<f32> {
    let extent = vec3<f32>(sdf.dims.xyz) * sdf.origin_voxel.w;
    return (p - sdf.origin_voxel.xyz) / extent;
}

fn sdf_distance(p: vec3<f32>) -> f32 {
    return textureSampleLevel(t_distance, s_linear, sdf_uvw(p), 0.0).r;
}

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

fn in_volume(p: vec3<f32>) -> bool {
    let uvw = sdf_uvw(p);
    return all(uvw >= vec3<f32>(0.0)) && all(uvw <= vec3<f32>(1.0));
}

fn hash_u32(x: u32) -> u32 {
    let s = x * 747796405u + 2891336453u;
    let word = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (word >> 22u) ^ word;
}

fn hash_unit(x: u32) -> f32 {
    return f32(hash_u32(x)) * (1.0 / 4294967296.0);
}

// Cone half-angle for a perceptual roughness (mirrors post::gi_reflect::cone_half_angle).
fn cone_half_angle(roughness: f32) -> f32 {
    return clamp(atan(roughness * roughness * 0.5), 0.01, 0.6);
}

// Schlick weight with F0 = mix(0.04, 1, metallic) and the smooth roughness range
// (mirrors post::gi_reflect::cache_weight).
fn cache_weight(roughness: f32, metallic: f32, n_dot_v: f32) -> f32 {
    let lo = params.min_roughness;
    let hi = params.max_roughness;
    let edge = 0.2 * (hi - lo);
    let range = smoothstep(lo, lo + edge, roughness) * (1.0 - smoothstep(hi - edge, hi, roughness));
    let f0 = 0.04 + 0.96 * metallic;
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - clamp(n_dot_v, 0.0, 1.0), 5.0);
    return clamp(range * fresnel * params.intensity, 0.0, MAX_WEIGHT);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(out_reflect);
    if (gid.x >= size.x || gid.y >= size.y) {
        return;
    }
    let px = vec2<i32>(gid.xy);
    let none = vec4<f32>(0.0);
    let nd = textureLoad(t_normal, px, 0);
    let surf = textureLoad(t_surface, px, 0);
    let roughness = surf.r;
    let metallic = surf.g;
    if (nd.a <= 0.0 || roughness < params.min_roughness || roughness > params.max_roughness) {
        textureStore(out_reflect, px, none);
        return;
    }
    // World position from the view depth (RH view space looks down -Z).
    let uv = (vec2<f32>(gid.xy) + 0.5) / params.screen;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let far = params.inv_proj * vec4<f32>(ndc, 1.0, 1.0);
    let dir_view = far.xyz / far.w;
    let view_z = nd.a * NORMAL_DEPTH_RANGE;
    let p_view = dir_view * (view_z / max(-dir_view.z, 1.0e-4));
    let world = (params.inv_view * vec4<f32>(p_view, 1.0)).xyz;
    let cam = params.inv_view[3].xyz;
    let n = normalize(nd.xyz);
    let v = normalize(cam - world);
    let weight = cache_weight(roughness, metallic, dot(n, v));
    if (weight <= 0.0 || !in_volume(world)) {
        textureStore(out_reflect, px, none);
        return;
    }
    // Reflected direction, spread inside the roughness cone (one ray; TAA averages the frames).
    let seed = gid.x * 1973u + gid.y * 9277u + params.frame * 26699u;
    let a = 2.0 * PI * hash_unit(seed);
    let r = tan(cone_half_angle(roughness)) * sqrt(hash_unit(seed ^ 0x9E3779B9u));
    var dir = reflect(-v, n);
    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(dir.y) > 0.9);
    let t1 = normalize(cross(helper, dir));
    let t2 = cross(dir, t1);
    let spread = normalize(dir + (t1 * cos(a) + t2 * sin(a)) * r);
    if (dot(spread, n) > 0.0) {
        dir = spread;
    }

    // Sphere trace from a point pushed off the surface.
    let eps = 0.5 * sdf.origin_voxel.w;
    let origin = world + n * (2.0 * sdf.origin_voxel.w);
    var t = 0.0;
    var hit = false;
    for (var i = 0u; i < MAX_STEPS; i = i + 1u) {
        if (t > params.max_distance) {
            break;
        }
        let p = origin + dir * t;
        if (!in_volume(p)) {
            break;
        }
        let d = sdf_distance(p);
        if (d < eps) {
            hit = true;
            break;
        }
        t = t + max(d, eps);
    }
    if (!hit) {
        textureStore(out_reflect, px, none);
        return;
    }
    let hit_pos = origin + dir * t;
    let hit_normal = sdf_normal(hit_pos);
    let surface = textureSampleLevel(t_albedo, s_linear, sdf_uvw(hit_pos), 0.0);
    var radiance = surface.rgb * surface.a * EMISSION_SCALE;
    radiance = radiance + surface.rgb * prev_irradiance(hit_pos, hit_normal) / PI;
    textureStore(out_reflect, px, vec4<f32>(radiance, weight));
}

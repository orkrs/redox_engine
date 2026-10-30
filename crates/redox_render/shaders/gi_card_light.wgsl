// Card light pass (GI-02 step 3): lights one quarter of the card atlas per frame.
//
// Dispatch: x and y cover the largest card, z = object * 6 + face over the table. Each invocation
// is one texel of one card; it runs only on rows where `row % 4 == frame % 4`, so every texel is
// lit once every four frames. A texel's point is rebuilt from the card (card_unproject) and its
// normal from the normal texture. Result, written to the radiance atlas:
//   radiance = emission + albedo / PI * (sun * max(n·l, 0) * visibility + points + E_prev(p, n)).
// Point lights are not shadowed (ASSUMPTION, cost); the sun uses a sphere trace of the global SDF.
//
// gi_common.wgsl is concatenated in front (shader/mod.rs): PI, GiGridGpu, CardObjectGpu, the
// previous-frame irradiance and the card projection helpers come from there.

struct CardLightParams {
    sun_dir: vec4<f32>,  // xyz unit direction towards the sun
    sun_rad: vec4<f32>,  // xyz sun radiance
    frame: vec4<u32>,    // x frame index, y cards table length, z point count
}

struct SdfInfo {
    origin_voxel: vec4<f32>, // volume minimum corner (xyz), voxel size (w)
    dims: vec4<u32>,         // voxel counts (xyz)
}

// Point light: position (xyz) and intensity (w); colour (rgb) and radius (w).
struct CardPoint {
    pos_intensity: vec4<f32>,
    color_radius: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: CardLightParams;
@group(0) @binding(1) var t_distance: texture_3d<f32>;
@group(0) @binding(2) var<uniform> sdf: SdfInfo;
@group(0) @binding(3) var s_linear: sampler;
@group(0) @binding(4) var t_card_albedo: texture_2d<f32>;
@group(0) @binding(5) var t_card_nd: texture_2d<f32>;
@group(0) @binding(6) var<storage, read> cards: array<CardObjectGpu>;
@group(0) @binding(7) var t_radiance: texture_storage_2d<rgba16float, write>;
@group(0) @binding(8) var<uniform> prev_grid: GiGridGpu;
@group(0) @binding(9) var t_prev_irradiance: texture_2d<f32>;
@group(0) @binding(10) var<storage, read> probe_offset: array<vec4<f32>>;
@group(0) @binding(11) var<storage, read> points: array<CardPoint>;

const EMISSION_SCALE: f32 = 16.0;
const SHADOW_STEPS: u32 = 32u;
const SHADOW_MAX_DISTANCE: f32 = 40.0;
const POINT_SLOTS: u32 = 8u;
const ROW_STRIDE: u32 = 4u;

fn sdf_uvw(p: vec3<f32>) -> vec3<f32> {
    let extent = vec3<f32>(sdf.dims.xyz) * sdf.origin_voxel.w;
    return (p - sdf.origin_voxel.xyz) / extent;
}

fn sdf_distance(p: vec3<f32>) -> f32 {
    return textureSampleLevel(t_distance, s_linear, sdf_uvw(p), 0.0).r;
}

// 1 when the ray from `p` along `dir` misses the scene, 0 when it hits (sphere tracing).
fn light_visibility(p: vec3<f32>, dir: vec3<f32>) -> f32 {
    let voxel = sdf.origin_voxel.w;
    var t = 0.0;
    for (var i = 0u; i < SHADOW_STEPS; i = i + 1u) {
        let d = sdf_distance(p + dir * t);
        if (d < 0.5 * voxel) {
            return 0.0;
        }
        t = t + max(d, 0.5 * voxel);
        if (t > SHADOW_MAX_DISTANCE) {
            break;
        }
    }
    return 1.0;
}

// Unshadowed contribution of point light `k` at `p` with normal `n`.
fn point_light(p: vec3<f32>, n: vec3<f32>, k: u32) -> vec3<f32> {
    let pl = points[k];
    let to = pl.pos_intensity.xyz - p;
    let d2 = max(dot(to, to), 0.01);
    let d = sqrt(d2);
    let r = max(pl.color_radius.w, 1.0e-3);
    let window = clamp(1.0 - pow(d / r, 4.0), 0.0, 1.0);
    let l = to / d;
    return pl.color_radius.rgb * pl.pos_intensity.w * (window * window / d2) * max(dot(n, l), 0.0);
}

// Sum over the POINT_SLOTS nearest point lights (insertion into a sorted fixed-size list).
fn nearest_points(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var best_d: array<f32, 8>;
    var best_i: array<u32, 8>;
    for (var j = 0u; j < POINT_SLOTS; j = j + 1u) {
        best_d[j] = 1.0e30;
    }
    let count = params.frame.z;
    for (var k = 0u; k < count; k = k + 1u) {
        let to = points[k].pos_intensity.xyz - p;
        let d = dot(to, to);
        if (d >= best_d[POINT_SLOTS - 1u]) {
            continue;
        }
        var pos = POINT_SLOTS - 1u;
        loop {
            if (pos == 0u) {
                break;
            }
            if (best_d[pos - 1u] <= d) {
                break;
            }
            best_d[pos] = best_d[pos - 1u];
            best_i[pos] = best_i[pos - 1u];
            pos = pos - 1u;
        }
        best_d[pos] = d;
        best_i[pos] = k;
    }
    var sum = vec3<f32>(0.0);
    for (var j = 0u; j < POINT_SLOTS; j = j + 1u) {
        if (best_d[j] < 1.0e29) {
            sum = sum + point_light(p, n, best_i[j]);
        }
    }
    return sum;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let slot = gid.z;
    let object = slot / 6u;
    let face = slot % 6u;
    if (object >= params.frame.y) {
        return;
    }
    let rect = cards[object].rects[face];
    if (rect.z == 0u || rect.w == 0u || gid.x >= rect.z || gid.y >= rect.w) {
        return;
    }
    if (gid.y % ROW_STRIDE != params.frame.x % ROW_STRIDE) {
        return;
    }
    let texel = vec2<i32>(i32(rect.x + gid.x), i32(rect.y + gid.y));
    let nd = textureLoad(t_card_nd, texel, 0);
    let code = u32(round(nd.a * 255.0));
    if (code >= 255u) {
        textureStore(t_radiance, texel, vec4<f32>(0.0));
        return;
    }
    // Depth code 0..254 maps to depth 0..1 (see card_capture::encode).
    let depth = f32(code) / 254.0;
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(f32(rect.z), f32(rect.w));
    let lo = cards[object].aabb_min.xyz;
    let hi = cards[object].aabb_max.xyz;
    let p = card_unproject(face, lo, hi, uv, depth);
    let raw = nd.xyz * 2.0 - 1.0;
    let n = select(card_face_normal(face), normalize(raw), length(raw) > 1.0e-4);

    let alb = textureLoad(t_card_albedo, texel, 0);
    let emission = alb.a * EMISSION_SCALE;
    let sun = params.sun_dir.xyz;
    var direct = vec3<f32>(0.0);
    let ndl = max(dot(n, sun), 0.0);
    if (ndl > 0.0) {
        let lit_from = p + n * sdf.origin_voxel.w;
        direct = params.sun_rad.xyz * ndl * light_visibility(lit_from, sun);
    }
    direct = direct + nearest_points(p, n);
    let indirect = prev_irradiance(p, n);
    let radiance = vec3<f32>(emission) + alb.rgb / PI * (direct + indirect);
    textureStore(t_radiance, texel, vec4<f32>(radiance, 1.0));
}

// Screen-space VSM SMRT: 1 Vogel ray / pixel (temporal pass accumulates).

struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

struct LightUniform {
    dir_color: vec4<f32>,
    dir_direction: vec4<f32>,
    ambient: vec4<f32>,
    shadow_view_proj: mat4x4<f32>,
    point_lights_pos: array<vec4<f32>, 128>,
    point_lights_color: array<vec4<f32>, 128>,
    num_point_lights: u32,
    soft_shadows: u32,
    point_light_path: u32,
    vsm_screen_shadows: u32,
};

struct VsmPassUniform {
    frame_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var t_normal_depth: texture_2d<f32>;
@group(0) @binding(1) var<uniform> camera: Camera;
@group(0) @binding(2) var<uniform> light_u: LightUniform;
@group(0) @binding(3) var t_vsm_pages: texture_2d_array<f32>;
@group(0) @binding(4) var s_vsm: sampler;
@group(0) @binding(5) var<storage, read> vsm_info: VsmGpuInfo;
@group(0) @binding(6) var t_blue_noise: texture_2d<f32>;
@group(0) @binding(7) var s_blue_noise: sampler;
@group(0) @binding(8) var<uniform> vsm_pass: VsmPassUniform;

const FAR: f32 = 1000.0;
const VSM_PAGE_VALID: u32 = 0x80000000u;
const VSM_PAGE_PHYS_MASK: u32 = 0x3FFFFFFFu;
const GOLDEN: f32 = 0.61803398875;

struct VsmLevelGpu {
    view_proj: mat4x4<f32>,
    world_half_extent: f32,
    page_table_offset: u32,
    pages_per_side: u32,
    texel_world: f32,
}

struct VsmGpuInfo {
    level_count: u32,
    page_px: u32,
    phys_page_count: u32,
    phys_pages_per_side: u32,
    ray_count: u32,
    samples_per_ray: u32,
    source_angle: f32,
    source_radius: f32,
    max_trace_dist: f32,
    _pad0: vec3<f32>,
    levels: array<VsmLevelGpu, 8>,
    page_tables: array<u32, 2048>,
}

struct VsmLookup {
    valid: f32,
    layer: i32,
    pixel: vec2<i32>,
    pixel_f: vec2<f32>,
    receiver_z: f32,
    z_world_per_ndc: f32,
    texel_world: f32,
};

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

fn view_pos_from_uv_depth(uv: vec2<f32>, linear_depth: f32) -> vec3<f32> {
    let view_z = max(linear_depth * FAR, 1e-4);
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let inv_m00 = 1.0 / max(camera.proj[0][0], 1e-6);
    let inv_m11 = 1.0 / max(camera.proj[1][1], 1e-6);
    return vec3<f32>(ndc.x * inv_m00 * view_z, ndc.y * inv_m11 * view_z, -view_z);
}

fn world_pos_from_uv_depth(uv: vec2<f32>, linear_depth: f32) -> vec3<f32> {
    let vp = view_pos_from_uv_depth(uv, linear_depth);
    return (camera.inv_view * vec4<f32>(vp, 1.0)).xyz;
}

fn vogel(i: u32, n: u32, phi: f32) -> vec2<f32> {
    let k = f32(i) + 0.5;
    let r = sqrt(k / f32(n));
    let t = k * 2.39996323 + phi;
    return vec2<f32>(cos(t), sin(t)) * r;
}

fn vsm_lookup(pos: vec3<f32>) -> VsmLookup {
    var out: VsmLookup;
    out.valid = 0.0;
    out.layer = 0;
    out.pixel = vec2<i32>(0);
    out.pixel_f = vec2<f32>(0.0);
    out.receiver_z = 0.0;
    out.z_world_per_ndc = 1.0;
    out.texel_world = 1.0;
    let count = min(vsm_info.level_count, 8u);
    for (var lv = 0u; lv < count; lv++) {
        let cm = vsm_info.levels[lv];
        let clip = cm.view_proj * vec4<f32>(pos, 1.0);
        if (clip.w <= 1.0e-4) { continue; }
        let ndc = clip.xyz / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) { continue; }
        if (ndc.z < 0.0 || ndc.z > 1.0) { continue; }
        let pps = f32(cm.pages_per_side);
        let page = min(vec2<u32>(uv * pps), vec2<u32>(cm.pages_per_side - 1u));
        let entry = vsm_info.page_tables[cm.page_table_offset + page.y * cm.pages_per_side + page.x];
        if ((entry & VSM_PAGE_VALID) == 0u) { continue; }
        let phys = entry & VSM_PAGE_PHYS_MASK;
        let page_uv = fract(uv * pps);
        let ppx = f32(vsm_info.page_px);
        let pf = 1.0 + clamp(page_uv, vec2<f32>(0.0), vec2<f32>(1.0)) * (ppx - 2.0);
        out.valid = 1.0;
        out.layer = i32(phys);
        out.pixel_f = pf;
        out.pixel = vec2<i32>(i32(pf.x), i32(pf.y));
        out.receiver_z = ndc.z;
        out.z_world_per_ndc = cm.world_half_extent * 4.0;
        out.texel_world = (cm.world_half_extent * 2.0) / (pps * ppx);
        return out;
    }
    return out;
}

fn vsm_tap_lit(lk: VsmLookup, pix: vec2<i32>, bias_ndc: f32, min_diff: f32) -> f32 {
    let lo = 1;
    let hi = i32(vsm_info.page_px) - 2;
    // Taps beyond the page interior replicate the edge texel; returning "lit" drew bright
    // seams along every page border.
    let tap = clamp(pix, vec2<i32>(lo), vec2<i32>(hi));
    let stored = textureLoad(t_vsm_pages, tap, lk.layer, 0).r;
    if (stored > 0.999) { return 1.0; }
    let occluded = stored < lk.receiver_z - bias_ndc && (lk.receiver_z - stored) > min_diff;
    return select(1.0, 0.0, occluded);
}

fn vsm_pcf_bilinear(lk: VsmLookup, bias_ndc: f32, min_diff: f32) -> f32 {
    let i0 = vec2<i32>(i32(floor(lk.pixel_f.x)), i32(floor(lk.pixel_f.y)));
    let a = fract(lk.pixel_f);
    let s00 = vsm_tap_lit(lk, i0, bias_ndc, min_diff);
    let s10 = vsm_tap_lit(lk, i0 + vec2<i32>(1, 0), bias_ndc, min_diff);
    let s01 = vsm_tap_lit(lk, i0 + vec2<i32>(0, 1), bias_ndc, min_diff);
    let s11 = vsm_tap_lit(lk, i0 + vec2<i32>(1, 1), bias_ndc, min_diff);
    return mix(mix(s00, s10, a.x), mix(s01, s11, a.x), a.y);
}

fn vsm_trace_1ray(world_pos: vec3<f32>, normal: vec3<f32>, light_dir: vec3<f32>, phi: f32) -> f32 {
    // Returns 1.0 (lit) when clipmap has no resident page — VSM-only policy:
    // outer regions are lit; SDF contact provides near shadowing.
    let f = normalize(light_dir);
    let lk0 = vsm_lookup(world_pos);
    if (lk0.valid < 0.5) { return 1.0; }
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(f.y) < 0.99);
    let tangent = normalize(cross(up, f));
    let bitangent = cross(f, tangent);
    let tan_source = tan(max(vsm_info.source_angle, 1e-5));
    let vog = vogel(0u, 1u, phi);
    let radius = tan_source * length(vog);
    let dir = normalize(f + tangent * vog.x * radius + bitangent * vog.y * radius);

    let n_dot_l = max(dot(normalize(normal), f), 0.0);
    let origin = world_pos + normalize(normal) * (lk0.texel_world * mix(0.25, 0.85, n_dot_l));
    let samples = clamp(vsm_info.samples_per_ray, 1u, 8u);
    var transmittance = 1.0;

    for (var s = 1u; s <= samples; s++) {
        if (transmittance < 0.01) { break; }
        let u = f32(s) / f32(samples);
        let dist = max(vsm_info.max_trace_dist * u * u * u * u, lk0.texel_world * 1.5);
        let sp = origin + dir * dist;
        let lk = vsm_lookup(sp);
        if (lk.valid < 0.5) { break; }
        let slope = sqrt(max(1.0 - abs(dot(normal, f)), 0.0));
        let contact = clamp(1.0 - dist / max(lk.texel_world * 16.0, 1e-4), 0.0, 1.0);
        let face_on = max(dot(normalize(normal), f), 0.0);
        let bias_ndc = lk.texel_world * (0.35 + 0.55 * slope + 0.9 * face_on)
            * mix(1.0, 0.45, contact) / lk.z_world_per_ndc;
        let spread = clamp(dist * tan_source / max(lk.texel_world, 1e-4), 1.0, 4.0);
        let lit = vsm_pcf_bilinear(lk, bias_ndc, (1.25 * lk.texel_world) / lk.z_world_per_ndc);
        let occ = 1.0 - lit;
        if (occ > 0.01) {
            let stored = textureLoad(t_vsm_pages, lk.pixel, lk.layer, 0).r;
            let depth_diff = max((lk.receiver_z - stored) * lk.z_world_per_ndc, 0.0);
            let penumbra = max(vsm_info.source_angle * dist, lk.texel_world);
            let soft = clamp(1.0 - depth_diff / penumbra, 0.0, 1.0);
            transmittance *= mix(1.0, soft, occ);
        }
    }
    return clamp(transmittance, 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) f32 {
    let dims = vec2<f32>(textureDimensions(t_normal_depth));
    let nd = textureLoad(t_normal_depth, vec2<i32>(in.uv * dims), 0);
    if (nd.a >= 0.999 || nd.a <= 1e-5) { return 1.0; }
    if (light_u.soft_shadows != 2u) { return 1.0; }

    let world_pos = world_pos_from_uv_depth(in.uv, nd.a);
    let normal = normalize(nd.rgb);
    let l_dir = normalize(light_u.dir_direction.xyz);

    let bn = textureSample(
        t_blue_noise,
        s_blue_noise,
        in.clip_pos.xy / vec2<f32>(textureDimensions(t_blue_noise)),
    ).r;
    let phi = fract(bn + f32(vsm_pass.frame_index) * GOLDEN) * 6.28318530718;

    return vsm_trace_1ray(world_pos, normal, l_dir, phi);
}

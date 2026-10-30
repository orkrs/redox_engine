// Froxel fog injection (PST-06 step 3). One invocation per froxel: writes the in-scatter
// (rgb, already multiplied by the extinction) and the extinction (a) of the slab centred at
// its depth. Mirrors fog_composite.wgsl and src/post/fog_froxel.rs; keep in sync.
// Sun shadows come from the VSM clipmap, copied from pbr.wgsl (vsm_lookup, vsm_pcf_bilinear).

struct FogUniform {
    color_density : vec4<f32>,   // rgb ambient colour, w density
    falloff       : vec4<f32>,   // x height falloff, y base height, z start distance, w max opacity
    scatter       : vec4<f32>,   // x sun scatter, y anisotropy, z near, w far
    camera_pos    : vec4<f32>,
    sun_dir       : vec4<f32>,   // direction the sun shines in (zero when unknown)
    sun_color     : vec4<f32>,   // sun colour times intensity
    inv_view_proj : mat4x4<f32>,
};

// Mirror of shadow::vsm_clipmap::VsmLevelGpu / VsmGpuInfo (same as pbr.wgsl).
const VSM_PAGE_VALID: u32 = 0x80000000u;
const VSM_PAGE_PHYS_MASK: u32 = 0x3FFFFFFFu;

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

const FROXEL_W: u32 = 160u;
const FROXEL_H: u32 = 90u;
const FROXEL_D: u32 = 64u;
const FROXEL_D_F: f32 = 64.0;
const FROXEL_MAX_DISTANCE: f32 = 120.0;
const PI: f32 = 3.14159265;

@group(0) @binding(0) var<uniform> fog : FogUniform;
@group(0) @binding(1) var vol_out : texture_storage_3d<rgba16float, write>;
@group(0) @binding(2) var<storage, read> vsm_info : VsmGpuInfo;
@group(0) @binding(3) var t_vsm_pages : texture_2d_array<f32>;

// Far end of the grid in view depth.
fn froxel_far() -> f32 {
    return min(fog.scatter.w, FROXEL_MAX_DISTANCE);
}

// View depth of slice boundary i (mirror of slice_depth in src/post/fog.rs).
fn slice_depth(i: f32) -> f32 {
    let near = fog.scatter.z;
    return near * pow(froxel_far() / near, i / FROXEL_D_F);
}

// Unit world-space direction of the view ray through a uv (as in fog.wgsl).
fn view_ray(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let a = fog.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let b = fog.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    return normalize(a.xyz / a.w - b.xyz / b.w);
}

fn hg_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    return (1.0 - g2) / (4.0 * PI * pow(denom, 1.5));
}

// Finest clipmap level whose window covers pos and whose page is resident (vsm_lookup in pbr.wgsl).
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
        if (clip.w <= 1.0e-4) {
            continue;
        }
        let ndc = clip.xyz / clip.w;
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            continue;
        }
        if (ndc.z < 0.0 || ndc.z > 1.0) {
            continue;
        }

        let pps = f32(cm.pages_per_side);
        let page = min(vec2<u32>(uv * pps), vec2<u32>(cm.pages_per_side - 1u));
        let entry =
            vsm_info.page_tables[cm.page_table_offset + page.y * cm.pages_per_side + page.x];
        if ((entry & VSM_PAGE_VALID) == 0u) {
            continue;
        }

        let phys = entry & VSM_PAGE_PHYS_MASK;
        let page_uv = fract(uv * pps);
        let ppx = f32(vsm_info.page_px);
        let lo = 1.0;
        let hi = ppx - 1.0;
        let pf = lo + clamp(page_uv, vec2<f32>(0.0), vec2<f32>(1.0)) * (hi - lo);

        out.valid = 1.0;
        out.layer = i32(phys);
        out.pixel_f = pf;
        out.pixel = vec2<i32>(i32(pf.x), i32(pf.y));
        out.receiver_z = ndc.z;
        out.z_world_per_ndc = cm.world_half_extent * 4.0;
        out.texel_world = (cm.world_half_extent * 2.0) / (pps * f32(vsm_info.page_px));
        return out;
    }
    return out;
}

fn vsm_tap_lit(lk: VsmLookup, pix: vec2<i32>, bias_ndc: f32, min_diff: f32) -> f32 {
    let lo = 1;
    let hi = i32(vsm_info.page_px) - 2;
    let tap = clamp(pix, vec2<i32>(lo), vec2<i32>(hi));
    let stored = textureLoad(t_vsm_pages, tap, lk.layer, 0).r;
    if (stored > 0.999) {
        return 1.0;
    }
    let occluded = stored < lk.receiver_z - bias_ndc
        && (lk.receiver_z - stored) > min_diff;
    return select(1.0, 0.0, occluded);
}

fn vsm_pcf_bilinear(lk: VsmLookup, bias_ndc: f32, min_diff: f32) -> f32 {
    let f = lk.pixel_f;
    let i0 = vec2<i32>(i32(floor(f.x)), i32(floor(f.y)));
    let a = fract(f);
    let s00 = vsm_tap_lit(lk, i0, bias_ndc, min_diff);
    let s10 = vsm_tap_lit(lk, i0 + vec2<i32>(1, 0), bias_ndc, min_diff);
    let s01 = vsm_tap_lit(lk, i0 + vec2<i32>(0, 1), bias_ndc, min_diff);
    let s11 = vsm_tap_lit(lk, i0 + vec2<i32>(1, 1), bias_ndc, min_diff);
    return mix(mix(s00, s10, a.x), mix(s01, s11, a.x), a.y);
}

// Sun visibility at a world point: 1 when lit, or when VSM is off or the point is not covered.
fn sun_visibility(p: vec3<f32>) -> f32 {
    if (fog.scatter.x <= 0.0 || vsm_info.level_count == 0u) {
        return 1.0;
    }
    let lk = vsm_lookup(p);
    if (lk.valid < 0.5) {
        return 1.0;
    }
    let min_diff = (1.25 * lk.texel_world) / lk.z_world_per_ndc;
    return vsm_pcf_bilinear(lk, 0.0, min_diff);
}

@compute @workgroup_size(4, 4, 4)
fn cs_inject(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= FROXEL_W || id.y >= FROXEL_H || id.z >= FROXEL_D) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(f32(FROXEL_W), f32(FROXEL_H));
    let ray = view_ray(uv);
    let fwd = view_ray(vec2<f32>(0.5, 0.5));
    let cos_t = max(dot(ray, fwd), 1e-4);
    // Slab centre: view depth z, then distance along the ray.
    let z = slice_depth(f32(id.z) + 0.5);
    let p = fog.camera_pos.xyz + ray * (z / cos_t);

    var sigma = 0.0;
    if (z >= fog.falloff.z) {
        sigma = fog.color_density.w * exp(-fog.falloff.x * (p.y - fog.falloff.y));
    }
    let phase = hg_phase(dot(ray, -fog.sun_dir.xyz), fog.scatter.y);
    let sun = fog.scatter.x * fog.sun_color.rgb * phase * sun_visibility(p);
    let scatter = sigma * (fog.color_density.rgb + sun);
    textureStore(vol_out, vec3<i32>(id), vec4<f32>(scatter, sigma));
}

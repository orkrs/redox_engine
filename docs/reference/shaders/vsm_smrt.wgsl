//! VSM clipmap lookup + SMRT (Shadow Map Ray Tracing) soft sampling.
//!
//! This file is the **canonical source** of the sampling helpers used by
//! `pbr.wgsl`'s `vsm_shadow()`. WGSL has no `#include`, so the block between
//! BEGIN/END markers below is duplicated verbatim at the end of `pbr.wgsl`;
//! keep the two copies in sync (a concat-based single source is possible
//! later via `shader::load`, which already joins strings).
//!
//! GPU resources (reserved bind group 25 in the PBR pipeline layout):
//! ```text
//! @group(0) @binding(19)  t_vsm_pages : texture_2d_array<f32>
//!      One LAYER per physical atlas page (128×128 R32Float each). Layer
//!      index == physical page index from the page table. NOTE: the device
//!      must support at least `pool_pages` array layers (default limit 2048;
//!      clamp VsmConfig::pool_pages accordingly when creating the texture).
//!      Texels hold NDC-z ([0,1]) of the owning clipmap level's ortho
//!      (matches shaders/vsm_depth.wgsl output).
//! @group(0) @binding(20)  s_vsm : sampler (linear)
//!      Reserved for future moment filtering; current sampling uses
//!      textureLoad because R32Float is not filterable without the
//!      FLOAT32_FILTERABLE feature.
//! @group(0) @binding(21)  vsm_info : storage read-only VsmGpuInfo
//!      Mirrored by Rust `shadow::vsm_clipmap::VsmGpuInfo` — see the layout
//!      contract there before touching any field order.
//! ```
//!
//! The function stays dead code (and its bindings unused, hence unvalidated
//! against the pipeline layout) until the renderer binds group 25 and calls
//! it from the shadow-mode branch.

// ── BEGIN VSM_SMRT BLOCK (mirrored into pbr.wgsl verbatim) ──────────────

/// Page-table VALID bit (mirror of `shadow::page_table::PAGE_VALID_BIT`).
const VSM_PAGE_VALID: u32 = 0x80000000u;
/// Page-table physical-index mask (mirror of `PAGE_PHYS_MASK`).
const VSM_PAGE_PHYS_MASK: u32 = 0x3FFFFFFFu;

/// GPU mirror of one clipmap level (`shadow::vsm_clipmap::VsmLevelGpu`).
struct VsmLevelGpu {
    view_proj: mat4x4<f32>,
    world_half_extent: f32,
    page_table_offset: u32,
    pages_per_side: u32,
    texel_world: f32,
}

/// GPU mirror of `shadow::vsm_clipmap::VsmGpuInfo`.
struct VsmGpuInfo {
    level_count: u32,
    page_px: u32,
    phys_page_count: u32,
    phys_pages_per_side: u32,
    // SMRT params (mirror of SmrtUniform / shadow::smrt::SmrtConfig).
    ray_count: u32,
    samples_per_ray: u32,
    source_angle: f32,
    source_radius: f32,
    max_trace_dist: f32,
    _pad0: vec3<f32>,
    levels: array<VsmLevelGpu, 8>,
    /// Concatenated page tables: `level * 256 + y * 16 + x`.
    page_tables: array<u32, 2048>,
}

@group(0) @binding(19) var t_vsm_pages: texture_2d_array<f32>;
@group(0) @binding(20) var s_vsm: sampler;
@group(0) @binding(21) var<storage, read> vsm_info: VsmGpuInfo;

/// Interleaved gradient noise (Jimenez 2014) for per-pixel ray rotation.
fn vsm_ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

/// Result of resolving a world position against the clipmap page tables.
struct VsmLookup {
    /// 1.0 when a valid page was found covering the position.
    valid: f32,
    /// Physical page index (== texture array layer).
    layer: i32,
    /// Pixel inside the 128² page.
    pixel: vec2<i32>,
    pixel_f: vec2<f32>,
    /// NDC depth of the query point in the owning level.
    receiver_z: f32,
    /// World units per unit of NDC depth at this level ((far − near)).
    z_world_per_ndc: f32,
    /// World units per shadow texel at this level.
    texel_world: f32,
};

/// Finds the finest clipmap level whose window covers `pos` AND whose page
/// table entry is valid. Coarser levels are tried in order.
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
            continue; // page not resident; try the coarser level
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
        // Level ortho spans near=−2h .. far=+2h ⇒ NDC depth unit = 4h world.
        out.z_world_per_ndc = cm.world_half_extent * 4.0;
        out.texel_world = (cm.world_half_extent * 2.0)
            / (pps * f32(vsm_info.page_px));
        return out;
    }
    return out;
}

fn vsm_tap_lit(lk: VsmLookup, pix: vec2<i32>, bias_ndc: f32, min_diff: f32) -> f32 {
    let lo = 1;
    let hi = i32(vsm_info.page_px) - 2;
    // Taps beyond the page interior replicate the edge texel. Returning "lit" here drew
    // bright seams along every page border (wider than the PCF kernel at coarse levels).
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

fn vsm_pcf_lit(lk: VsmLookup, bias_ndc: f32, spread: f32) -> f32 {
    let min_diff = (1.25 * lk.texel_world) / lk.z_world_per_ndc;
    if (spread <= 1.5) {
        return vsm_pcf_bilinear(lk, bias_ndc, min_diff);
    }
    if (spread <= 2.5) {
        var acc = vsm_pcf_bilinear(lk, bias_ndc, min_diff);
        acc += vsm_tap_lit(lk, vec2<i32>(i32(lk.pixel_f.x), i32(lk.pixel_f.y)) + vec2<i32>(-1, 0), bias_ndc, min_diff);
        acc += vsm_tap_lit(lk, vec2<i32>(i32(lk.pixel_f.x), i32(lk.pixel_f.y)) + vec2<i32>(1, 0), bias_ndc, min_diff);
        acc += vsm_tap_lit(lk, vec2<i32>(i32(lk.pixel_f.x), i32(lk.pixel_f.y)) + vec2<i32>(0, -1), bias_ndc, min_diff);
        acc += vsm_tap_lit(lk, vec2<i32>(i32(lk.pixel_f.x), i32(lk.pixel_f.y)) + vec2<i32>(0, 1), bias_ndc, min_diff);
        return acc / 5.0;
    }
    let sp = max(spread, 1.0);
    var acc = 0.0;
    var wsum = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let w = f32(2 - abs(dx)) * f32(2 - abs(dy));
            let pf = lk.pixel_f + vec2<f32>(f32(dx), f32(dy)) * sp;
            let i0 = vec2<i32>(i32(floor(pf.x)), i32(floor(pf.y)));
            let a = fract(pf);
            let s00 = vsm_tap_lit(lk, i0, bias_ndc, min_diff);
            let s10 = vsm_tap_lit(lk, i0 + vec2<i32>(1, 0), bias_ndc, min_diff);
            let s01 = vsm_tap_lit(lk, i0 + vec2<i32>(0, 1), bias_ndc, min_diff);
            let s11 = vsm_tap_lit(lk, i0 + vec2<i32>(1, 1), bias_ndc, min_diff);
            acc += mix(mix(s00, s10, a.x), mix(s01, s11, a.x), a.y) * w;
            wsum += w;
        }
    }
    return acc / max(wsum, 1.0);
}

/// SMRT-style soft shadow for the directional light.
///
/// Traces `ray_count` rays from the shading point toward the light disc
/// (angular radius `source_angle`), marching `samples_per_ray` steps each.
/// Steps use a cubic distance curve so the first samples sit centimetres
/// from the receiver (contact at caster bases). Each step is 3×3 PCF'd.
///
/// Returns visibility in [0, 1]. Positions not covered by any resident page
/// return 1.0 (lit) so the virtualized area acts purely as an optimization.
fn vsm_shadow(world_pos: vec3<f32>, normal: vec3<f32>, light_dir: vec3<f32>) -> f32 {
    let count = min(vsm_info.level_count, 8u);
    if (count == 0u) {
        return 1.0;
    }

    let f = normalize(light_dir); // direction toward the light
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(f.y) < 0.99);
    let tangent = normalize(cross(up, f));
    let bitangent = cross(f, tangent);

    let ray_count = clamp(vsm_info.ray_count, 1u, 16u);
    let samples_per_ray = clamp(vsm_info.samples_per_ray, 1u, 16u);
    let tan_source = tan(max(vsm_info.source_angle, 1e-5));

    let jitter = vsm_ign(world_pos.xy + world_pos.zz * 7.31);

    let lk0 = vsm_lookup(world_pos);
    var origin = world_pos;
    if (lk0.valid > 0.5) {
        let n_dot_l = max(dot(normalize(normal), f), 0.0);
        let push = lk0.texel_world * mix(0.6, 1.5, n_dot_l);
        origin = world_pos + normalize(normal) * push;
    }

    var sum = 0.0;

    for (var r = 0u; r < ray_count; r++) {
        let u1 = (f32(r) + 0.5 + jitter * 0.5) / f32(ray_count);
        let theta = fract(u1 * 2.39996323 + jitter) * 6.28318530718;
        let radius = tan_source * sqrt(u1);
        let dir = normalize(
            f + (tangent * cos(theta) + bitangent * sin(theta)) * radius,
        );

        var transmittance = 1.0;
        for (var s = 1u; s <= samples_per_ray; s++) {
            if (transmittance < 0.01) {
                break;
            }
            let u = f32(s) / f32(samples_per_ray);
            let dist = max(vsm_info.max_trace_dist * u * u * u * u, lk0.texel_world * 1.5);
            let sp = origin + dir * dist;

            let lk = vsm_lookup(sp);
            if (lk.valid < 0.5) {
                break;
            }

            let slope = sqrt(max(1.0 - abs(dot(normal, f)), 0.0));
            let contact = clamp(1.0 - dist / max(lk.texel_world * 16.0, 1e-4), 0.0, 1.0);
            let face_on = max(dot(normalize(normal), f), 0.0);
            let bias_ndc = lk.texel_world * (0.45 + 0.60 * slope + 1.0 * face_on)
                * mix(1.0, 0.45, contact) / lk.z_world_per_ndc;
            // Minimum radius of 2 texels: a hard 1-texel bilinear edge shows texel stair-steps.
            let spread = clamp(dist * tan_source / max(lk.texel_world, 1e-4), 2.0, 8.0);
            let lit = vsm_pcf_lit(lk, bias_ndc, spread);
            let occ = 1.0 - lit;
            if (occ > 0.01) {
                let stored = textureLoad(t_vsm_pages, lk.pixel, lk.layer, 0).r;
                let depth_diff = max((lk.receiver_z - stored) * lk.z_world_per_ndc, 0.0);
                let penumbra = max(vsm_info.source_angle * dist, lk.texel_world);
                let soft = clamp(1.0 - depth_diff / penumbra, 0.0, 1.0);
                transmittance *= mix(1.0, soft, occ);
            }
        }
        sum += transmittance;
    }
    return clamp(sum / f32(ray_count), 0.0, 1.0);

// ── END VSM_SMRT BLOCK ───────────────────────────────────────────────────

// Standalone validation entry point (unused; lets tools parse this file
// independently of pbr.wgsl).
fn _vsm_smrt_validate(pos: vec3<f32>, n: vec3<f32>, l: vec3<f32>) -> f32 {
    return vsm_shadow(pos, n, l);
}

struct Camera {
    // Must mirror rust CameraUniform exactly (464 bytes).
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

struct LightUniform {
    dir_color:      vec4<f32>,
    dir_direction:  vec4<f32>,
    ambient:        vec4<f32>,
    shadow_view_proj: mat4x4<f32>,
    point_lights_pos:   array<vec4<f32>, 128>,
    point_lights_color: array<vec4<f32>, 128>,
    num_point_lights:   u32,
    soft_shadows:       u32,
    point_light_path:   u32,
    vsm_screen_shadows: u32,
};

struct Material {
    base_color: vec4<f32>,
    emissive:   vec4<f32>,
    metallic:   f32,
    roughness:  f32,
    flags:      u32,
    sheen:      f32,
    // Substrate lobes (layer0/1): rgb + metallic; params = roughness, weight, pad, pad.
    layer0_color: vec4<f32>,
    layer0_params: vec4<f32>,
    layer1_color: vec4<f32>,
    layer1_params: vec4<f32>,
};

struct Model {
    model: mat4x4<f32>,
    // Inverse-transpose of `model` (upper-left 3x3), computed on CPU.
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

// ── Group 0: Frame / global (rare updates) ──────────────────────────────
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> shader_debug: ShaderDebugUniform;
@group(0) @binding(2) var t_irradiance: texture_cube<f32>;
@group(0) @binding(3) var t_prefiltered: texture_cube<f32>;
@group(0) @binding(4) var t_brdf_lut: texture_2d<f32>;
@group(0) @binding(5) var s_ibl: sampler;
@group(0) @binding(6) var t_blue_noise: texture_2d<f32>;
@group(0) @binding(7) var s_blue_noise: sampler;

// ── Group 1: Pass / scene (once per pass) ───────────────────────────────
@group(1) @binding(0) var<uniform> light_u: LightUniform;
@group(1) @binding(1) var t_shadow: texture_depth_2d;
@group(1) @binding(2) var s_shadow: sampler_comparison;
@group(1) @binding(3) var t_gtao: texture_2d<f32>;
@group(1) @binding(4) var s_gtao: sampler;
@group(1) @binding(5) var<storage, read> point_lights: array<PointLight>;
@group(1) @binding(6) var<storage, read> cluster_metadata: array<ClusterMetadata>;
@group(1) @binding(7) var<storage, read> cluster_light_indices: array<u32>;
@group(1) @binding(8) var<uniform> cluster_info: ClusterInfo;
@group(1) @binding(11) var t_local_shadow_atlas: texture_depth_2d;
@group(1) @binding(13) var t_vsm_pages: texture_2d_array<f32>;
@group(1) @binding(14) var s_vsm: sampler;
@group(1) @binding(15) var<storage, read> vsm_info: VsmGpuInfo;

// ── DDGI probe grid (Phase 5) ───────────────────────────────────────────
// Real: trilinear 8-probe blend + octahedral irradiance lookup.
// Missing: GPU ray-tracing / probe visibility (CPU fills samples).
struct DdgiGridGpu {
    origin: vec4<f32>,
    spacing: f32,
    dims_x: u32,
    dims_y: u32,
    dims_z: u32,
    enabled: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}
struct DdgiProbeGpu {
    // 8 octahedral irradiance samples; validity in samples[0].w
    samples: array<vec4<f32>, 8>,
}
@group(1) @binding(16) var<uniform> ddgi_grid: DdgiGridGpu;
@group(1) @binding(17) var<storage, read> ddgi_probes: array<DdgiProbeGpu>;

// ── SDF brick atlas (Phase 5) ───────────────────────────────────────────
struct SdfBrickPageGpu {
    world_min: vec4<f32>,
    world_max: vec4<f32>,
    atlas_origin_res: vec4<f32>, // xyz texel origin, w = brick res (64)
}
struct SdfSceneGpu {
    atlas_dims: vec4<u32>, // x = brick count, w = brick res
    world_bounds_min: vec4<f32>,
    world_bounds_max_texel: vec4<f32>, // xyz union max, w = texel size
    pages: array<SdfBrickPageGpu, 8>,
}
@group(1) @binding(18) var<uniform> sdf_scene: SdfSceneGpu;
@group(1) @binding(19) var t_sdf_bricks: texture_3d<f32>;
@group(1) @binding(20) var s_sdf_bricks: sampler;
@group(1) @binding(21) var t_vsm_shadow: texture_2d<f32>;
@group(1) @binding(22) var s_vsm_shadow: sampler;

// ── Group 2: Materials ──────────────────────────────────────────────────
@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var t_albedo: texture_2d<f32>;
@group(2) @binding(2) var t_normal: texture_2d<f32>;
@group(2) @binding(3) var t_mr: texture_2d<f32>;
@group(2) @binding(4) var s_mat: sampler;

// ── Group 3: Instances ──────────────────────────────────────────────────
@group(3) @binding(0) var<storage, read> models: array<Model>;

// Clustered Forward Rendering structures
struct PointLight {
    position: vec4<f32>,
    color: vec4<f32>,
    // Shadow view-projection matrices for point-light cube faces.
    // Face order: PosX, NegX, PosY, NegY, PosZ, NegZ (matches CPU).
    shadow_matrices: array<mat4x4<f32>, 6>,
    intensity: f32,
    radius: f32,
    // First atlas tile of this light's 6-tile run (NO_TILE = 0xFFFFFFFFu when
    // the light casts no shadow or the atlas saturated).
    shadow_tile_base: u32,
    _pad: f32,
};

struct ClusterMetadata {
    offset: u32,
    count: u32,
};

struct ClusterInfo {
    clusters_x: u32,
    clusters_y: u32,
    depth_slices: u32,
    screen_width: u32,
    screen_height: u32,
    near: f32,
    far: f32,
    depth_scale: f32,
    total_clusters: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct ShaderDebugUniform {
    debug_viz_mode: u32,
    frame_index: u32,
    ibl_enabled: u32,
    gtao_enabled: u32,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos:    vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) world_tangent: vec3<f32>,
    @location(3) uv:           vec2<f32>,
    @location(4) instance_color: vec4<f32>,
    @location(5) shadow_pos:     vec4<f32>,
};

@vertex
fn vs_main(in: CompressedVertIn, @builtin(instance_index) instance_idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let mdl = models[instance_idx];
    let bounds_min = mdl.bounds_min.xyz;
    let bounds_extent = mdl.bounds_extent.xyz;
    let local_pos = decode_local_position(in.pos_q, bounds_min, bounds_extent);
    let model = mdl.model;
    let world_pos = model * vec4<f32>(local_pos, 1.0);
    out.clip_pos = camera.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    let nm = mdl.normal_matrix;
    let n = normalize(nm * oct_decode(in.normal_oct));
    var t = normalize(nm * oct_decode(in.tangent_oct));
    t = normalize(t - n * dot(n, t));
    out.world_normal = n;
    out.world_tangent = t * decode_tangent_sign(in.pos_q);
    out.uv = in.uv;
    out.instance_color = mdl.color;
    out.shadow_pos = light_u.shadow_view_proj * world_pos;
    return out;
}

const PI: f32 = 3.14159265359;

/// Drop NaN/Inf and clamp HDR before bloom/TAA (a few 1e4+ pixels flood the screen).
fn sanitize_hdr(c: vec3<f32>) -> vec3<f32> {
    if (any(c != c) || any(abs(c) > vec3<f32>(65504.0))) {
        return vec3<f32>(0.0);
    }
    // Practical scene peak well below fp16 max — stops bloom from turning
    // one over-bright speck into a full-frame white/cyan flash.
    return clamp(c, vec3<f32>(0.0), vec3<f32>(64.0));
}

fn oct_encode(n: vec3<f32>) -> vec2<f32> {
    var v = n / max(abs(n.x) + abs(n.y) + abs(n.z), 1.0e-6);
    if (v.y < 0.0) {
        let ox = v.x;
        v.x = (1.0 - abs(v.z)) * select(-1.0, 1.0, ox >= 0.0);
        v.z = (1.0 - abs(ox)) * select(-1.0, 1.0, v.z >= 0.0);
    }
    return clamp(v.xz * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
}

fn ddgi_sample_at(probe: DdgiProbeGpu, idx: u32) -> vec3<f32> {
    // Naga rejects dynamic indexing into uniform/struct arrays — unroll.
    switch idx {
        case 0u: { return probe.samples[0].xyz; }
        case 1u: { return probe.samples[1].xyz; }
        case 2u: { return probe.samples[2].xyz; }
        case 3u: { return probe.samples[3].xyz; }
        case 4u: { return probe.samples[4].xyz; }
        case 5u: { return probe.samples[5].xyz; }
        case 6u: { return probe.samples[6].xyz; }
        default: { return probe.samples[7].xyz; }
    }
}

fn ddgi_probe_oct(probe: DdgiProbeGpu, n: vec3<f32>) -> vec3<f32> {
    // 2×4 octahedral atlas bilinear sample.
    let uv = oct_encode(n);
    let fx = uv.x * 2.0 - 0.5;
    let fy = uv.y * 4.0 - 0.5;
    let x0 = u32(clamp(floor(fx), 0.0, 1.0));
    let y0 = u32(clamp(floor(fy), 0.0, 3.0));
    let x1 = min(x0 + 1u, 1u);
    let y1 = min(y0 + 1u, 3u);
    let tx = fract(fx);
    let ty = fract(fy);
    let s00 = ddgi_sample_at(probe, y0 * 2u + x0);
    let s10 = ddgi_sample_at(probe, y0 * 2u + x1);
    let s01 = ddgi_sample_at(probe, y1 * 2u + x0);
    let s11 = ddgi_sample_at(probe, y1 * 2u + x1);
    return mix(mix(s00, s10, tx), mix(s01, s11, tx), ty);
}

fn ddgi_probe_index(x: i32, y: i32, z: i32) -> u32 {
    return u32(z) * ddgi_grid.dims_x * ddgi_grid.dims_y
        + u32(y) * ddgi_grid.dims_x
        + u32(x);
}

fn ddgi_sample_irradiance(world_pos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if (ddgi_grid.enabled == 0u) {
        return vec3(0.0);
    }
    let dims = vec3<i32>(
        i32(ddgi_grid.dims_x),
        i32(ddgi_grid.dims_y),
        i32(ddgi_grid.dims_z),
    );
    if (any(dims <= vec3<i32>(0)) || ddgi_grid.spacing <= 0.0) {
        return vec3(0.0);
    }
    let local = (world_pos - ddgi_grid.origin.xyz) / ddgi_grid.spacing;
    let base = floor(local);
    let frac = local - base;
    var accum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dz = 0; dz < 2; dz = dz + 1) {
        for (var dy = 0; dy < 2; dy = dy + 1) {
            for (var dx = 0; dx < 2; dx = dx + 1) {
                let ix = i32(base.x) + dx;
                let iy = i32(base.y) + dy;
                let iz = i32(base.z) + dz;
                if (ix < 0 || iy < 0 || iz < 0 || ix >= dims.x || iy >= dims.y || iz >= dims.z) {
                    continue;
                }
                let probe = ddgi_probes[ddgi_probe_index(ix, iy, iz)];
                let validity = probe.samples[0].w;
                let wx = select(frac.x, 1.0 - frac.x, dx == 0);
                let wy = select(frac.y, 1.0 - frac.y, dy == 0);
                let wz = select(frac.z, 1.0 - frac.z, dz == 0);
                let w = wx * wy * wz * validity;
                accum = accum + ddgi_probe_oct(probe, n) * w;
                wsum = wsum + w;
            }
        }
    }
    if (wsum > 1.0e-5) {
        return accum / wsum;
    }
    return vec3(0.0);
}

const SDF_CONE_APERTURE: f32 = 0.25;
const SDF_MAX_CONE_STEPS: u32 = 32u;
const SDF_ATLAS_EXTENT: f32 = 128.0;

fn sdf_load_atlas(coord: vec3<i32>) -> f32 {
    let c = clamp(coord, vec3<i32>(0), vec3<i32>(i32(SDF_ATLAS_EXTENT) - 1));
    // R32Float is non-filterable — use textureLoad (NonFiltering sampler on binding 28).
    return textureLoad(t_sdf_bricks, c, 0).r;
}

fn sdf_sample_page(page: SdfBrickPageGpu, p: vec3<f32>) -> f32 {
    let span = page.world_max.xyz - page.world_min.xyz;
    let local = (p - page.world_min.xyz) / max(span, vec3<f32>(1.0e-6));
    if (any(local < vec3<f32>(0.0)) || any(local > vec3<f32>(1.0))) {
        return 1.0e9;
    }
    let res = page.atlas_origin_res.w;
    let origin = page.atlas_origin_res.xyz;
    let grid = origin + local * res - vec3<f32>(0.5);
    let base = floor(grid);
    let f = grid - base;
    let b = vec3<i32>(base);
    var accum = 0.0;
    for (var z = 0; z < 2; z = z + 1) {
        for (var y = 0; y < 2; y = y + 1) {
            for (var x = 0; x < 2; x = x + 1) {
                let w = select(f.x, 1.0 - f.x, x == 0)
                    * select(f.y, 1.0 - f.y, y == 0)
                    * select(f.z, 1.0 - f.z, z == 0);
                accum = accum + sdf_load_atlas(b + vec3<i32>(x, y, z)) * w;
            }
        }
    }
    return accum;
}

fn sdf_aabb(p: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> f32 {
    let c = (bmin + bmax) * 0.5;
    let h = (bmax - bmin) * 0.5;
    let q = abs(p - c) - h;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn sdf_sample_world(p: vec3<f32>) -> f32 {
    let count = sdf_scene.atlas_dims.x;
    if (count == 0u) {
        return 1.0e9;
    }
    var best = 1.0e9;
    for (var i = 0u; i < count; i = i + 1u) {
        let page = sdf_scene.pages[i];
        let d_box = sdf_aabb(p, page.world_min.xyz, page.world_max.xyz);
        if (d_box <= 0.0) {
            best = min(best, sdf_sample_page(page, p));
        } else {
            // Conservative step toward this brick so traces cannot skip pages.
            best = min(best, d_box);
        }
    }
    return best;
}

fn sdf_trace_shadow(world_pos: vec3<f32>, light_dir: vec3<f32>, normal: vec3<f32>) -> vec2<f32> {
    if (sdf_scene.atlas_dims.x == 0u) {
        return vec2<f32>(1.0, 1.0e9);
    }
    let texel = sdf_scene.world_bounds_max_texel.w;
    // Keep bias modest — stacked 2×texel normal + light push caused peter-panning.
    let bias = 0.75 * texel;
    let origin = world_pos + normal * bias;
    let dir = normalize(light_dir);
    let max_dist = length(sdf_scene.world_bounds_max_texel.xyz - sdf_scene.world_bounds_min.xyz);
    let half_tan = tan(0.5 * SDF_CONE_APERTURE);
    let t_min = max(0.5 * texel, 1.0e-4);
    var visibility = 1.0;
    var contact_dist = max_dist;
    var min_sd = 1.0e9;
    var t = t_min;
    for (var step = 0u; step < SDF_MAX_CONE_STEPS; step = step + 1u) {
        if (t >= max_dist) {
            break;
        }
        let p = origin + dir * t;
        let sd = sdf_sample_world(p);
        min_sd = min(min_sd, sd);
        let r = half_tan * t;
        let occ = select(
            clamp((r - sd) / max(r, 1.0e-6), 0.0, 1.0),
            select(1.0, 0.0, sd <= 0.0),
            r > 1.0e-6,
        );
        let dt = select(
            clamp(abs(sd), t_min, max(r, t_min)),
            t_min,
            sd > 0.0,
        );
        let cov = min(dt / max(r, t_min), 1.0);
        let prev_vis = visibility;
        visibility = visibility * (1.0 - occ * cov);
        if (visibility < prev_vis && contact_dist > t) {
            contact_dist = t;
        }
        if (visibility < 1.0e-3) {
            break;
        }
        t = t + dt;
    }
    if (contact_dist >= max_dist) {
        contact_dist = max(min_sd, 0.0);
    }
    return vec2<f32>(clamp(visibility, 0.0, 1.0), contact_dist);
}

fn fetch_shadow(shadow_coords: vec4<f32>) -> f32 {
    // Perspective divide
    let sc = shadow_coords.xyz / shadow_coords.w;
    // NDC (-1 to 1) to texture (0 to 1), flip Y
    let proj_coords = vec3<f32>(
        sc.x * 0.5 + 0.5,
        sc.y * -0.5 + 0.5,
        sc.z
    );
    
    if (proj_coords.z > 1.0) {
        return 1.0;
    }

    var visibility = 0.0;
    let size = 1.0 / 2048.0; // Shadow map resolution
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * size;
            visibility += textureSampleCompare(
                t_shadow, s_shadow, 
                proj_coords.xy + offset, 
                proj_coords.z - 0.005 // bias
            );
        }
    }
    return visibility / 9.0;
}

fn cube_face_index(dir: vec3<f32>) -> u32 {
    let ad = abs(dir);
    if (ad.x >= ad.y && ad.x >= ad.z) {
        return select(1u, 0u, dir.x >= 0.0); // NegX : PosX
    }
    if (ad.y >= ad.x && ad.y >= ad.z) {
        return select(3u, 2u, dir.y >= 0.0); // NegY : PosY
    }
    return select(5u, 4u, dir.z >= 0.0);     // NegZ : PosZ
}

fn select_face_matrix(mats: array<mat4x4<f32>, 6>, face: u32) -> mat4x4<f32> {
    // Naga/WGSL validation can be strict about dynamically indexing arrays of matrices.
    // Use a switch with constant indices for maximum compatibility.
    switch (face) {
        case 0u: { return mats[0]; }
        case 1u: { return mats[1]; }
        case 2u: { return mats[2]; }
        case 3u: { return mats[3]; }
        case 4u: { return mats[4]; }
        default: { return mats[5]; }
    }
}

fn get_point_light_shadow(light: PointLight, world_pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    // Lights without an atlas run (saturated or cast_shadows=false) are lit.
    if (light.shadow_tile_base == 0xFFFFFFFFu) {
        return 1.0;
    }

    let dir = world_pos - light.position.xyz;
    let face = cube_face_index(dir);
    let shadow_matrix = select_face_matrix(light.shadow_matrices, face);

    // Normal bias to fight acne.
    let bias_world_pos = world_pos + normal * 0.02;
    let clip = shadow_matrix * vec4<f32>(bias_world_pos, 1.0);
    if (clip.w <= 1.0e-4) {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let local_uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);

    if (local_uv.x < 0.0 || local_uv.x > 1.0 || local_uv.y < 0.0 || local_uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
        return 1.0;
    }

    // ── Tile mapping: tile grid is TILES_PER_AXIS × TILES_PER_AXIS (16×16) ──
    let tpa = 16.0; // TILES_PER_AXIS as f32
    let tile_index = light.shadow_tile_base + face;
    let col = f32(tile_index % 16u);
    let row = floor(f32(tile_index) / tpa);
    let inv_tpa = 1.0 / tpa;

    // 5×5 PCF — monolithic look, no grain.
    let texel_atlas = inv_tpa / 512.0; // TILE_PX = 512
    let tile_origin = vec2<f32>(col, row) * inv_tpa;
    var visibility = 0.0;
    for (var y = -2; y <= 2; y = y + 1) {
        for (var x = -2; x <= 2; x = x + 1) {
            // local_uv [0,1] → atlas UV inside this tile, then offset.
            var sample_uv = tile_origin + local_uv * inv_tpa
                + vec2<f32>(f32(x), f32(y)) * texel_atlas;
            sample_uv = clamp(
                sample_uv,
                tile_origin + texel_atlas,
                tile_origin + inv_tpa - texel_atlas,
            );
            // Level-variant: this fn runs under non-uniform control flow
            // (per-light loop), implicit derivatives are invalid.
            visibility += textureSampleCompareLevel(
                t_local_shadow_atlas, s_shadow,
                sample_uv,
                ndc.z - 0.002,
            );
        }
    }
    return visibility / 25.0;
}

fn shadow_sample_dither(clip_xy: vec2<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(t_blue_noise));
    let noise = textureSample(t_blue_noise, s_blue_noise, clip_xy / dims).r;
    return fract(noise + f32(shader_debug.frame_index % 16u) * 0.61803398875);
}

fn sample_vsm_shadow(screen_uv: vec2<f32>) -> f32 {
    return textureSample(t_vsm_shadow, s_vsm_shadow, screen_uv).r;
}

fn multi_bounce_gtao(raw_ao: f32, albedo: vec3<f32>) -> f32 {
    // Jimenez GTAO multi-bounce polynomial (GDC 2016) tinted by albedo luminance.
    // Filament reference: filament/shaders/src/ambient_occlusion.fs
    let a = 2.0404 * raw_ao - 0.3324;
    let b = -4.7951 * raw_ao + 0.6417;
    let c = 2.7552 * raw_ao + 0.5869;
    let bounce = ((raw_ao * a + b) * raw_ao + c) * raw_ao;
    let lum = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
    let albedo_term = clamp(lum, 0.0, 1.0);
    return mix(raw_ao, max(raw_ao, bounce), albedo_term);
}

fn apply_geometric_specular_aa(normal: vec3<f32>, roughness: f32) -> f32 {
    // Decima / Toksvig-style: both screen-space normal derivatives (not max-only).
    let dndx = dpdx(normal);
    let dndy = dpdy(normal);
    let variance = 0.5 * (dot(dndx, dndx) + dot(dndy, dndy));
    let kernel_roughness = clamp(variance, 0.0, 0.25);
    return sqrt(roughness * roughness + kernel_roughness);
}

fn distribution_ggx(n: vec3<f32>, h: vec3<f32>, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let n_dot_h = max(dot(n, h), 0.0);
    let n_dot_h2 = n_dot_h * n_dot_h;
    let nom = a2;
    let denom = (n_dot_h2 * (a2 - 1.0) + 1.0);
    return nom / (PI * denom * denom);
}

fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let r = (roughness + 1.0);
    let k = (r * r) / 8.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, roughness: f32) -> f32 {
    let n_dot_v = max(dot(n, v), 0.0);
    let n_dot_l = max(dot(n, l), 0.0);
    let ggx2 = geometry_schlick_ggx(n_dot_v, roughness);
    let ggx1 = geometry_schlick_ggx(n_dot_l, roughness);
    return ggx1 * ggx2;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

fn fresnel_schlick_roughness(cos_theta: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    return f0 + (max(vec3<f32>(1.0 - roughness), f0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

/// Cook-Torrance BRDF × n·l for one Substrate / base lobe.
fn eval_brdf_lobe(
    n: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    roughness: f32,
) -> vec3<f32> {
    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 0.0);
    if (n_dot_l <= 0.0 || n_dot_v <= 0.0) {
        return vec3<f32>(0.0);
    }
    let vl = v + l;
    let vl_sq = dot(vl, vl);
    if (vl_sq < 1.0e-8) {
        return vec3<f32>(0.0);
    }
    let h = vl * inverseSqrt(vl_sq);
    let r = apply_geometric_specular_aa(n, clamp(roughness, 0.05, 1.0));
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let ndf = distribution_ggx(n, h, r);
    let g = geometry_smith(n, v, l, r);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
    let specular = (ndf * g * f) / max(4.0 * n_dot_v * n_dot_l, 0.0001);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    return (kd * albedo / PI + specular) * n_dot_l;
}

/// Soft fabric sheen (Charlie-like) — weight from `material.sheen`.
fn eval_sheen_lobe(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, sheen_w: f32) -> vec3<f32> {
    if (sheen_w <= 1.0e-4) {
        return vec3<f32>(0.0);
    }
    let vl = v + l;
    let vl_sq = dot(vl, vl);
    if (vl_sq < 1.0e-8) {
        return vec3<f32>(0.0);
    }
    let h = vl * inverseSqrt(vl_sq);
    let n_dot_h = max(dot(n, h), 0.0);
    let n_dot_l = max(dot(n, l), 0.0);
    let sin2 = max(1.0 - n_dot_h * n_dot_h, 0.0);
    // Soft inverted-Gaussian lobe toward grazing normals.
    let d = pow(sin2, 2.5);
    return vec3<f32>(sheen_w) * d * n_dot_l;
}

/// Weighted Substrate multi-lobe evaluation (base + up to two layers + sheen).
fn eval_substrate_lit(
    n: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    roughness: f32,
) -> vec3<f32> {
    let w0 = material.layer0_params.y;
    let w1 = material.layer1_params.y;
    let w_base = max(1.0 - w0 - w1, 0.0);
    var out = w_base * eval_brdf_lobe(n, v, l, albedo, metallic, roughness);
    if (w0 > 1.0e-4) {
        out = out + w0 * eval_brdf_lobe(
            n, v, l,
            material.layer0_color.xyz,
            material.layer0_color.w,
            material.layer0_params.x,
        );
    }
    if (w1 > 1.0e-4) {
        out = out + w1 * eval_brdf_lobe(
            n, v, l,
            material.layer1_color.xyz,
            material.layer1_color.w,
            material.layer1_params.x,
        );
    }
    out = out + eval_sheen_lobe(n, v, l, material.sheen);
    return out;
}

// Clustered Forward Rendering helper functions
fn linearize_depth(ndc_depth: f32, near: f32, far: f32) -> f32 {
    let z_ndc = ndc_depth * 2.0 - 1.0; // Convert from [0, 1] to [-1, 1]
    return (2.0 * near * far) / ((far + near) - z_ndc * (far - near));
}

fn get_depth_slice(linear_depth: f32, cluster_info: ClusterInfo) -> u32 {
    let z_norm = log(linear_depth / cluster_info.near) / cluster_info.depth_scale;
    return min(u32(max(z_norm, 0.0)), cluster_info.depth_slices - 1u);
}

fn get_cluster_index(screen_pos: vec2<f32>, cluster_info: ClusterInfo) -> u32 {
    let cluster_x = u32(screen_pos.x / 16.0); // CLUSTER_SIZE_X = 16
    let cluster_y = u32(screen_pos.y / 16.0); // CLUSTER_SIZE_Y = 16
    
    if (cluster_x >= cluster_info.clusters_x || cluster_y >= cluster_info.clusters_y) {
        return 0xFFFFFFFFu; // Invalid index
    }
    
    return cluster_x + cluster_y * cluster_info.clusters_x;
}

/// Per-fragment inputs of the material surface function.
struct SurfaceInput {
    world_pos: vec3<f32>,
    world_normal: vec3<f32>,
    world_tangent: vec3<f32>,
    uv: vec2<f32>,
    view_dir: vec3<f32>,
    instance_color: vec4<f32>,
    frag_coord: vec4<f32>,
};

/// Everything the lighting code reads from a material surface.
struct SurfaceData {
    base_color: vec3<f32>,
    opacity: f32,
    normal: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
    // Substrate lobes: layer rgb + metallic, then params (roughness, weight, pad, pad).
    layer0: vec4<f32>,
    layer0_params: vec4<f32>,
    layer1: vec4<f32>,
    layer1_params: vec4<f32>,
    sheen: f32,
};

/// Material surface of one fragment: textures, alpha test and the shading normal and
/// roughness. The body is the former `fs_main` prologue moved verbatim (same operation order).
fn material_surface(in: SurfaceInput) -> SurfaceData {
    var albedo = material.base_color.rgb * in.instance_color.rgb;
    var alpha = material.base_color.a * in.instance_color.a;
    if ((material.flags & 1u) != 0u) {
        let tex = textureSample(t_albedo, s_mat, in.uv);
        albedo *= tex.rgb;
        alpha *= tex.a;
    }

    // Stochastic alpha-to-coverage with blue noise + golden-ratio temporal offset.
    let use_alpha_test = (material.flags & 16u) != 0u;
    if (use_alpha_test || alpha < 0.999) {
        let screen_uv = in.frag_coord.xy / vec2<f32>(textureDimensions(t_blue_noise));
        let noise = textureSample(t_blue_noise, s_blue_noise, screen_uv).r;
        let dither = fract(noise + f32(shader_debug.frame_index % 16u) * 0.61803398875);
        if (alpha < dither) {
            discard;
        }
    }

    var n = in.world_normal;
    let n_len_sq = dot(n, n);
    if (n_len_sq > 1.0e-12) {
        n = n * inverseSqrt(n_len_sq);
    } else {
        n = vec3<f32>(0.0, 1.0, 0.0);
    }
    var toksvig_delta = 0.0;
    if ((material.flags & 2u) != 0u) {
        let t = normalize(in.world_tangent);
        let b = cross(n, t);
        let tbn = mat3x3<f32>(t, b, n);
        let normal_sample = textureSample(t_normal, s_mat, in.uv);
        let normal_map = normal_sample.xyz * 2.0 - 1.0;
        // Alpha of Toksvig-baked normal mips stores roughness delta in [0,1]
        // mapped to [0, 0.5] (see resource/toksvig.rs).
        toksvig_delta = normal_sample.a * 0.5;
        n = normalize(tbn * normal_map);
    }

    // Geometric normal of the visible face. If the interpolated/CPU normal
    // points into the mesh, lighting looks sun-side-dark / shade-side-bright.
    // `normalize(cross(0))` is NaN and poisons the whole HDR buffer via bloom/TAA.
    let d_cross = cross(dpdx(in.world_pos), dpdy(in.world_pos));
    let d_len_sq = dot(d_cross, d_cross);
    if (d_len_sq > 1.0e-8) {
        var n_geo = d_cross * inverseSqrt(d_len_sq);
        let to_cam = camera.camera_pos.xyz - in.world_pos;
        if (dot(n_geo, to_cam) < 0.0) {
            n_geo = -n_geo;
        }
        // Only flip when clearly inverted — soft threshold avoids diagonal
        // triangulation seams (dpdx/dpdy jumps across the quad split).
        if (dot(n, n_geo) < -0.55) {
            n = -n;
        }
    }

    var metallic = material.metallic;
    var roughness = material.roughness;
    if ((material.flags & 4u) != 0u) {
        let mr = textureSample(t_mr, s_mat, in.uv);
        metallic *= mr.b;
        roughness *= mr.g;
    }
    roughness = clamp(roughness + toksvig_delta, 0.05, 1.0);

    return SurfaceData(
        albedo,
        alpha,
        n,
        metallic,
        roughness,
        material.emissive.rgb,
        material.layer0_color,
        material.layer0_params,
        material.layer1_color,
        material.layer1_params,
        material.sheen,
    );
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let v = normalize(camera.camera_pos.xyz - in.world_pos);
    let surf = material_surface(SurfaceInput(
        in.world_pos,
        in.world_normal,
        in.world_tangent,
        in.uv,
        v,
        in.instance_color,
        in.clip_pos,
    ));
    let albedo = surf.base_color;
    let n = surf.normal;
    let metallic = surf.metallic;
    let roughness = surf.roughness;
    let spec_roughness = apply_geometric_specular_aa(n, roughness);

    let n_dot_v = max(dot(n, v), 0.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // --- DEBUG VISUALIZATION OVERLAY ---
    if (shader_debug.debug_viz_mode == 1u) {
        let cluster_x = u32(in.clip_pos.x / 16.0);
        let cluster_y = u32(in.clip_pos.y / 16.0);
        let view_pos = camera.view * vec4<f32>(in.world_pos, 1.0);
        let view_z = max(-view_pos.z, cluster_info.near);
        let cluster_z = get_depth_slice(view_z, cluster_info);
        let cluster_idx_flat = cluster_x + cluster_y * cluster_info.clusters_x + cluster_z * cluster_info.clusters_x * cluster_info.clusters_y;
        let color_val = f32(cluster_idx_flat % 128u) / 127.0;
        return vec4<f32>(color_val, color_val * 0.5, 0.0, 1.0);
    } else if (shader_debug.debug_viz_mode == 2u) {
        // ShadowValue debug: combined directional visibility.
        let view_z = -(camera.view * vec4<f32>(in.world_pos, 1.0)).z;
        let l_dir = normalize(light_u.dir_direction.xyz);
        var s = 1.0;
        if (light_u.soft_shadows == 2u) {
            if (light_u.vsm_screen_shadows != 0u) {
                let screen_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(t_vsm_shadow));
                s = sample_vsm_shadow(screen_uv);
            } else {
                s = vsm_shadow(in.world_pos, n, l_dir);
            }
        }
        return vec4<f32>(s, s, s, 1.0);
    } else if (shader_debug.debug_viz_mode == 3u) {
        // ShadowSources: each darkening pipeline as its own colour.
        //   Magenta = GTAO
        //   Lime    = VSM screen shadow
        //   Yellow  = SDF cone trace
        let screen_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(t_gtao));
        let ao = textureSample(t_gtao, s_gtao, screen_uv).a;
        let ao_dark = clamp(1.0 - ao, 0.0, 1.0);

        let view_z = -(camera.view * vec4<f32>(in.world_pos, 1.0)).z;
        let l_dir = normalize(light_u.dir_direction.xyz);

        var vsm_dark = 0.0;
        if (light_u.soft_shadows == 2u) {
            var vsm_vis: f32;
            if (light_u.vsm_screen_shadows != 0u) {
                let vsm_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(t_vsm_shadow));
                vsm_vis = sample_vsm_shadow(vsm_uv);
            } else {
                vsm_vis = vsm_shadow(in.world_pos, n, l_dir);
            }
            vsm_dark = clamp(1.0 - vsm_vis, 0.0, 1.0);
        }

        let sdf_result = sdf_trace_shadow(in.world_pos, l_dir, n);
        let sdf_dark = clamp(1.0 - sdf_result.x, 0.0, 1.0);

        let base = albedo * 0.18;
        let colored =
            base
            + vec3<f32>(1.0, 0.15, 0.85) * ao_dark   // magenta — SSAO
            + vec3<f32>(0.45, 1.0, 0.2) * vsm_dark   // lime    — VSM
            + vec3<f32>(1.0, 0.92, 0.15) * sdf_dark; // yellow  — SDF
        return vec4<f32>(colored, 1.0);
    }

    // Directional shadow factor (filled in directional light block).
    var shadow = 1.0;
    var lo = vec3<f32>(0.0);

    // --- Directional Light — VSM-only ---
    {
        let l = normalize(light_u.dir_direction.xyz);
        let radiance = light_u.dir_color.rgb * light_u.dir_color.w;
        var dir_shadow: f32;
        if (light_u.vsm_screen_shadows != 0u) {
            let screen_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(t_vsm_shadow));
            dir_shadow = sample_vsm_shadow(screen_uv);
        } else {
            let lk = vsm_lookup(in.world_pos);
            if (lk.valid > 0.5) {
                dir_shadow = vsm_shadow(in.world_pos, n, l);
            } else {
                dir_shadow = 1.0;
            }
        }
        let sdf_result = sdf_trace_shadow(
            in.world_pos,
            l,
            n,
        );
        let blend = smoothstep(0.0, 0.3, sdf_result.y);
        shadow = mix(sdf_result.x, dir_shadow, blend);
        lo += eval_substrate_lit(n, v, l, albedo, metallic, roughness) * radiance * shadow;
    }

    // --- Point Lights (Clustered Forward Rendering) ---
    if (light_u.point_light_path == 0u) {
    let cluster_x = u32(in.clip_pos.x / 16.0);
    let cluster_y = u32(in.clip_pos.y / 16.0);
    
    // View-space Z (planar depth) — NOT euclidean distance (which draws circles on the floor).
    let view_pos = camera.view * vec4<f32>(in.world_pos, 1.0);
    let view_z = max(-view_pos.z, cluster_info.near);
    let cluster_z = get_depth_slice(view_z, cluster_info);
    
    // Get cluster index in the grid
    if (cluster_x < cluster_info.clusters_x && cluster_y < cluster_info.clusters_y && cluster_z < cluster_info.depth_slices) {
        let cluster_idx = cluster_x + cluster_y * cluster_info.clusters_x + cluster_z * cluster_info.clusters_x * cluster_info.clusters_y;
        
        // Get light list for this cluster
        let cluster_meta = cluster_metadata[cluster_idx];
        let light_count = arrayLength(&point_lights);
        let index_count = arrayLength(&cluster_light_indices);
        let n_cluster = min(cluster_meta.count, 256u);

        for (var i = 0u; i < n_cluster; i = i + 1u) {
            let idx_pos = cluster_meta.offset + i;
            if (idx_pos >= index_count) { break; }
            let light_idx = cluster_light_indices[idx_pos];
            if (light_idx >= light_count) { continue; }
            let light = point_lights[light_idx];
            
            let p_pos = light.position.xyz;
            let p_color = light.color.rgb;
            let p_intensity = light.intensity;
            let p_radius = light.radius;

            let to_light = p_pos - in.world_pos;
            let dist_sq = dot(to_light, to_light);
            let l = normalize(to_light);
            // Inverse-square with smooth Frostbite window — no hard rings.
            let d = sqrt(dist_sq);
            let inv_dist_sq = 1.0 / max(dist_sq, 0.01);
            let x = d / max(p_radius, 0.001);
            let x2 = x * x;
            let window = clamp(1.0 - x2 * x2, 0.0, 1.0);
            let attenuation = inv_dist_sq * window * window;
            let radiance = p_color * p_intensity;
            let p_shadow = get_point_light_shadow(light, in.world_pos, n);
            lo += eval_substrate_lit(n, v, l, albedo, metallic, roughness)
                * radiance * attenuation * p_shadow;
        }
    }
    }

    // GTAO (bent normal + AO) — sample before IBL so diffuse uses bent_n.
    let screen_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(t_gtao));
    let gtao = textureSample(t_gtao, s_gtao, screen_uv);
    // Use explicit uniform flag (shader_debug.gtao_enabled) — heuristic
    // gtao.a>=0.999 fails on flat open surfaces where AO naturally =1.
    let gtao_disabled = shader_debug.gtao_enabled == 0u;
    let bent_n = select(normalize(gtao.rgb), n, gtao_disabled);
    let raw_ao = select(gtao.a, 1.0, gtao_disabled);
    let ao = multi_bounce_gtao(raw_ao, albedo);
    // --- Ambient IBL ---
    let kS_ibl = fresnel_schlick_roughness(n_dot_v, f0, spec_roughness);
    let kD_ibl = (vec3<f32>(1.0) - kS_ibl) * (1.0 - metallic);

    var irradiance = vec3<f32>(0.0);
    var prefiltered_color = vec3<f32>(0.0);
    var env_brdf = vec2<f32>(0.0, 0.0);
    let r = reflect(-v, n);
    if (shader_debug.ibl_enabled != 0u) {
        irradiance = textureSample(t_irradiance, s_ibl, bent_n).rgb;
        prefiltered_color = textureSampleLevel(t_prefiltered, s_ibl, r, spec_roughness * 4.0).rgb;
        env_brdf = textureSample(t_brdf_lut, s_ibl, vec2<f32>(n_dot_v, spec_roughness)).rg;
    }
    let diffuse_ibl = irradiance * albedo;
    let specular_ibl = prefiltered_color * (kS_ibl * env_brdf.x + env_brdf.y);

    // Substrate layer IBL (weighted diffuse-ish contribution).
    var substrate_ibl = vec3<f32>(0.0);
    let w0 = material.layer0_params.y;
    let w1 = material.layer1_params.y;
    if (w0 > 1.0e-4 && shader_debug.ibl_enabled != 0u) {
        let a0 = material.layer0_color.xyz;
        let m0 = material.layer0_color.w;
        let r0 = clamp(material.layer0_params.x, 0.05, 1.0);
        let f0_0 = mix(vec3<f32>(0.04), a0, m0);
        let ks0 = fresnel_schlick_roughness(n_dot_v, f0_0, r0);
        let kd0 = (vec3<f32>(1.0) - ks0) * (1.0 - m0);
        let pre0 = textureSampleLevel(t_prefiltered, s_ibl, r, r0 * 4.0).rgb;
        let brdf0 = textureSample(t_brdf_lut, s_ibl, vec2<f32>(n_dot_v, r0)).rg;
        substrate_ibl = substrate_ibl + w0 * (kd0 * irradiance * a0 + pre0 * (ks0 * brdf0.x + brdf0.y));
    }
    if (w1 > 1.0e-4 && shader_debug.ibl_enabled != 0u) {
        let a1 = material.layer1_color.xyz;
        let m1 = material.layer1_color.w;
        let r1 = clamp(material.layer1_params.x, 0.05, 1.0);
        let f0_1 = mix(vec3<f32>(0.04), a1, m1);
        let ks1 = fresnel_schlick_roughness(n_dot_v, f0_1, r1);
        let kd1 = (vec3<f32>(1.0) - ks1) * (1.0 - m1);
        let pre1 = textureSampleLevel(t_prefiltered, s_ibl, r, r1 * 4.0).rgb;
        let brdf1 = textureSample(t_brdf_lut, s_ibl, vec2<f32>(n_dot_v, r1)).rg;
        substrate_ibl = substrate_ibl + w1 * (kd1 * irradiance * a1 + pre1 * (ks1 * brdf1.x + brdf1.y));
    }
    let w_base = max(1.0 - w0 - w1, 0.0);

    // Ambient factor from light uniform
    let ambient_factor = light_u.ambient.w;

    // Combine IBL with DDGI diffuse bounce (bent normal) — single ambient source.
    // Constant ambient (light_u.ambient.xyz) is small 0.02×albedo fallback when IBL/DDGI missing, not triple.
    let ddgi = ddgi_sample_irradiance(in.world_pos, bent_n);
    let ambient = (w_base * (kD_ibl * diffuse_ibl + specular_ibl) + substrate_ibl + ddgi * albedo + light_u.ambient.xyz * albedo) * ambient_factor * ao;
    let color = ambient + lo + material.emissive.rgb;
    return vec4<f32>(sanitize_hdr(color), material.base_color.a * in.instance_color.a);
}

// ── Virtual Shadow Maps: clipmap lookup + SMRT soft sampling ────────────
// Canonical source: docs/reference/shaders/vsm_smrt.wgsl (WGSL has no includes; this block
// is mirrored verbatim — keep both copies in sync).
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
    /// Pixel inside the page (integer, for raw loads).
    pixel: vec2<i32>,
    /// Sub-texel page coordinate for bilinear PCF.
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
            continue; // page not resident; try the coarser level
        }

        let phys = entry & VSM_PAGE_PHYS_MASK;
        let page_uv = fract(uv * pps);
        // Inset 1 texel: matches page_view_proj overlap, skips clear-border.
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

/// Tiered PCF: 4 taps near contact, 9 weighted taps mid-range, full 3×3 far.
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
/// Steps use a quadratic curve: dense near the receiver (contact) and
/// still reaching the far occluder so the umbra tip can soften.
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

    // Receiver offset along the geometric normal only. Any push toward the
    // light slides the umbra off the caster base (peter-panning).
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
}

// ── END VSM_SMRT BLOCK ───────────────────────────────────────────────────

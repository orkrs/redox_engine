// ── Virtual Geometry: VISIBILITY BUFFER RESOLVE PASS ─────────────────────
//
// Fullscreen pass that shades the visibility buffer written by vg_vis.wgsl
// with the SAME Cook-Torrance + IBL + clustered-lights + shadow pipeline as
// pbr.wgsl (all of it copied verbatim — WGSL has no includes).
//
// Per pixel:
//   1. RT0 id != SENTINEL? else `discard` (background / occluded pixels keep
//      whatever the earlier passes wrote).
//   2. Reconstruct world position from the hardware depth buffer with
//      camera.inv_proj / camera.inv_view.
//   3. Normal from RT1 (world space), UV from RT2.
//   4. material = materials[meshlets[instances[inst].meshlet_offset]
//         .material_index]   (one material per VG asset — see build_vg_asset)
//      stored in a flat `array<Material>` built by
//      `VgResolvePass::build_material_storage` on the Rust side; layout is
//      byte-identical to the per-object `MaterialUniform` (48 bytes).
//
// MATERIAL TEXTURE LIMITATION: WGSL cannot index bind groups, so per-material
// texture binding is impossible without bindless. The resolve therefore shades
// with MATERIAL-LEVEL parameters only: albedo = base_color, normal = flat,
// metallic/roughness = scalar factors. Texture flags in `Material.flags` are
// intentionally ignored here.
//
// Bindings:
//   group(0) = PBR frame bind group (camera / IBL / shader debug)
//   group(1) = PBR scene bind group (lights / shadows / clusters / VSM)
//   group(2) = this pass' own attachments + VG storage buffers.

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

/// Storage-buffer variant of the forward Material uniform (112 bytes,
/// identical field layout to `material::MaterialUniform`).
struct Material {
    base_color: vec4<f32>,
    emissive:   vec4<f32>,
    metallic:   f32,
    roughness:  f32,
    flags:      u32,
    sheen:      f32,
    layer0_color: vec4<f32>,
    layer0_params: vec4<f32>,
    layer1_color: vec4<f32>,
    layer1_params: vec4<f32>,
};

/// Mirror of `virtual_geometry::meshlet::VGInstanceData` (112 bytes).
struct VGInstance {
    transform: mat4x4<f32>,
    meshlet_offset: u32,
    meshlet_count: u32,
    flags: u32,
    _pad: u32,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

/// Mirror of `virtual_geometry::meshlet::MeshletDescriptor` (64 bytes).
struct MeshletDescriptor {
    vertex_offset: u32,
    vertex_count: u32,
    index_offset: u32,
    index_count: u32,

    sphere_center: vec3<f32>,
    sphere_radius: f32,

    cone_axis: vec3<f32>,
    cone_cutoff: f32,

    lod_error: f32,
    lod_level: u32,
    material_index: u32,
    _pad: u32,
};

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

struct ShadowUniform {
    // 4 cascade matrices (4×4 each)
    csm_matrices: array<mat4x4<f32>, 4>,
    // Split depths in view space
    csm_splits: vec4<f32>,
    // Size of the shadow atlas (e.g. 4096.0)
    shadow_atlas_size: f32,
    soft_near: f32,
    soft_far: f32,
    _pad2: u32,
};

struct ShaderDebugUniform {
    debug_viz_mode: u32,
    frame_index: u32,
    ibl_enabled: u32,
    gtao_enabled: u32,
};

// ── group(0): frame / global (mirrors pbr.wgsl group 0) ─────────────────

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> shader_debug: ShaderDebugUniform;
@group(0) @binding(2) var t_irradiance: texture_cube<f32>;
@group(0) @binding(3) var t_prefiltered: texture_cube<f32>;
@group(0) @binding(4) var t_brdf_lut: texture_2d<f32>;
@group(0) @binding(5) var s_ibl: sampler;
@group(0) @binding(6) var t_blue_noise: texture_2d<f32>;
@group(0) @binding(7) var s_blue_noise: sampler;

// ── group(1): pass / scene (mirrors pbr.wgsl group 1) ───────────────────

@group(1) @binding(0) var<uniform> light_u: LightUniform;
@group(1) @binding(1) var t_shadow: texture_depth_2d;
@group(1) @binding(2) var s_shadow: sampler_comparison;
@group(1) @binding(3) var t_ssao: texture_2d<f32>;
@group(1) @binding(4) var s_ssao: sampler;
@group(1) @binding(5) var<storage, read> point_lights: array<PointLight>;
@group(1) @binding(6) var<storage, read> cluster_metadata: array<ClusterMetadata>;
@group(1) @binding(7) var<storage, read> cluster_light_indices: array<u32>;
@group(1) @binding(8) var<uniform> cluster_info: ClusterInfo;
@group(1) @binding(9) var t_csm: texture_depth_2d_array;
@group(1) @binding(10) var s_csm: sampler_comparison;
@group(1) @binding(11) var t_local_shadow_atlas: texture_depth_2d;
@group(1) @binding(12) var<uniform> shadow_u: ShadowUniform;
@group(1) @binding(13) var t_vsm_pages: texture_2d_array<f32>;
@group(1) @binding(14) var s_vsm: sampler;
@group(1) @binding(15) var<storage, read> vsm_info: VsmGpuInfo;

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
    samples: array<vec4<f32>, 8>,
}
struct SdfBrickPageGpu {
    world_min: vec4<f32>,
    world_max: vec4<f32>,
    atlas_origin_res: vec4<f32>,
}
struct SdfSceneGpu {
    atlas_dims: vec4<u32>,
    world_bounds_min: vec4<f32>,
    world_bounds_max_texel: vec4<f32>,
    pages: array<SdfBrickPageGpu, 8>,
}
@group(1) @binding(16) var<uniform> ddgi_grid: DdgiGridGpu;
@group(1) @binding(17) var<storage, read> ddgi_probes: array<DdgiProbeGpu>;
@group(1) @binding(18) var<uniform> sdf_scene: SdfSceneGpu;
@group(1) @binding(19) var t_sdf_bricks: texture_3d<f32>;
@group(1) @binding(20) var s_sdf_bricks: sampler;
@group(1) @binding(21) var t_vsm_shadow: texture_2d<f32>;
@group(1) @binding(22) var s_vsm_shadow: sampler;

// ── group(2): vis-buffer attachments + VG storage ───────────────────────

@group(2) @binding(0) var t_vis_id:   texture_2d<u32>;        // RT0 r32uint
@group(2) @binding(1) var t_vis_nd:   texture_2d<f32>;        // RT1 rgba16float
@group(2) @binding(2) var t_vis_uv:   texture_2d<f32>;        // RT2 rg16float
@group(2) @binding(3) var t_depth:    texture_depth_2d;       // main depth
@group(2) @binding(4) var<storage, read> vg_instances: array<VGInstance>;
@group(2) @binding(5) var<storage, read> meshlets: array<MeshletDescriptor>;
@group(2) @binding(6) var<storage, read> vg_materials: array<Material>;

/// RT0 clear value written by the vis pass for "no VG geometry".
const SENTINEL: u32 = 0xFFFFFFFFu;

const PI: f32 = 3.14159265359;

fn sanitize_hdr(c: vec3<f32>) -> vec3<f32> {
    if (any(c != c) || any(abs(c) > vec3<f32>(65504.0))) {
        return vec3<f32>(0.0);
    }
    return clamp(c, vec3<f32>(0.0), vec3<f32>(64.0));
}

// ── BRDF helpers (verbatim from pbr.wgsl) ─────────────────────────────────

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

fn apply_geometric_specular_aa(normal: vec3<f32>, roughness: f32) -> f32 {
    let dndx = dpdx(normal);
    let dndy = dpdy(normal);
    let variance = 0.5 * (dot(dndx, dndx) + dot(dndy, dndy));
    let kernel_roughness = clamp(variance, 0.0, 0.25);
    return sqrt(roughness * roughness + kernel_roughness);
}

// ── Substrate multi-lobe evaluation (mirrors pbr.wgsl) ────────────────────
// Roughness is passed in already specular-AA'd: dpdx/dpdy must run in uniform
// control flow, so the caller computes it once before any light loop.

/// Cook-Torrance BRDF x n.l for one lobe.
fn eval_brdf_lobe(
    n: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    r: f32,
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
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let ndf = distribution_ggx(n, h, r);
    let g = geometry_smith(n, v, l, r);
    let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
    let specular = (ndf * g * f) / max(4.0 * n_dot_v * n_dot_l, 0.0001);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metallic);
    return (kd * albedo / PI + specular) * n_dot_l;
}

/// Soft fabric sheen (Charlie-like), weight from the material's sheen field.
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
    let d = pow(sin2, 2.5);
    return vec3<f32>(sheen_w) * d * n_dot_l;
}

/// Weighted Substrate evaluation: base + up to two layers + sheen.
/// lobe_r = (base, layer0, layer1) specular-AA'd roughness.
fn eval_substrate_lit(
    mat: Material,
    n: vec3<f32>,
    v: vec3<f32>,
    l: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    lobe_r: vec3<f32>,
) -> vec3<f32> {
    let w0 = mat.layer0_params.y;
    let w1 = mat.layer1_params.y;
    let w_base = max(1.0 - w0 - w1, 0.0);
    var out = w_base * eval_brdf_lobe(n, v, l, albedo, metallic, lobe_r.x);
    if (w0 > 1.0e-4) {
        out = out + w0 * eval_brdf_lobe(n, v, l, mat.layer0_color.xyz, mat.layer0_color.w, lobe_r.y);
    }
    if (w1 > 1.0e-4) {
        out = out + w1 * eval_brdf_lobe(n, v, l, mat.layer1_color.xyz, mat.layer1_color.w, lobe_r.z);
    }
    out = out + eval_sheen_lobe(n, v, l, mat.sheen);
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
    if (light.shadow_tile_base == 0xFFFFFFFFu) {
        return 1.0;
    }

    let dir = world_pos - light.position.xyz;
    let face = cube_face_index(dir);
    let shadow_matrix = select_face_matrix(light.shadow_matrices, face);

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

    let tpa = 16.0;
    let tile_index = light.shadow_tile_base + face;
    let col = f32(tile_index % 16u);
    let row = floor(f32(tile_index) / tpa);
    let inv_tpa = 1.0 / tpa;

    // 5×5 PCF — matches directional `csm_sample` look (monolithic, no grain).
    let texel_atlas = inv_tpa / 512.0; // TILE_PX = 512
    let tile_origin = vec2<f32>(col, row) * inv_tpa;
    var visibility = 0.0;
    for (var y = -2; y <= 2; y = y + 1) {
        for (var x = -2; x <= 2; x = x + 1) {
            var sample_uv = tile_origin + local_uv * inv_tpa
                + vec2<f32>(f32(x), f32(y)) * texel_atlas;
            sample_uv = clamp(
                sample_uv,
                tile_origin + texel_atlas,
                tile_origin + inv_tpa - texel_atlas,
            );
            visibility += textureSampleCompareLevel(
                t_local_shadow_atlas, s_shadow,
                sample_uv,
                ndc.z - 0.002,
            );
        }
    }
    return visibility / 25.0;
}

fn get_cascade_index(view_z: f32) -> u32 {
    var idx: u32 = 3u;
    for (var i = 0u; i < 4u; i = i + 1u) {
        if (view_z < shadow_u.csm_splits[i]) {
            idx = i;
            break;
        }
    }
    return idx;
}

fn csm_sample(cascade_idx: u32, uv: vec2<f32>, depth: f32) -> f32 {
    var visibility = 0.0;
    let texel_size = 1.0 / shadow_u.shadow_atlas_size;
    for (var y = -2; y <= 2; y = y + 1) {
        for (var x = -2; x <= 2; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            visibility += textureSampleCompareLevel(
                t_csm, s_csm, uv + offset, i32(cascade_idx), depth,
            );
        }
    }
    return visibility / 25.0;
}

fn linearize_depth01(d: f32) -> f32 {
    let denom = shadow_u.soft_far - d * (shadow_u.soft_far - shadow_u.soft_near);
    if (abs(denom) < 1e-6) { return shadow_u.soft_far; }
    return shadow_u.soft_near * shadow_u.soft_far / denom;
}

fn shadow_sample_dither(clip_xy: vec2<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(t_blue_noise));
    let noise = textureSample(t_blue_noise, s_blue_noise, clip_xy / dims).r;
    return fract(noise + f32(shader_debug.frame_index % 16u) * 0.61803398875);
}

fn pcss_csm(world_pos: vec3<f32>, view_z: f32, normal: vec3<f32>, light_angle: f32, dither: f32) -> f32 {
    let cascade_idx = get_cascade_index(view_z);
    let mat = shadow_u.csm_matrices[cascade_idx];
    let biased = world_pos + normal * 0.0004;
    let clip = mat * vec4<f32>(biased, 1.0);
    if (clip.w <= 1.0e-4) {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }

    let texel = 1.0 / shadow_u.shadow_atlas_size;

    var blocker_sum = 0.0;
    var blocker_count = 0.0;
    let search_radius = texel * 4.0;
    let atlas_res = vec2<f32>(shadow_u.shadow_atlas_size);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * search_radius;
            let coord = vec2<i32>((uv + offset) * atlas_res);
            let d = textureLoad(t_csm, coord, i32(cascade_idx), 0);
            if (d < ndc.z) {
                blocker_sum += d;
                blocker_count += 1.0;
            }
        }
    }
    if (blocker_count < 0.5) {
        return 1.0;
    }
    let blocker_ndc = blocker_sum / blocker_count;

    let receiver_d = linearize_depth01(ndc.z);
    let blocker_d = linearize_depth01(blocker_ndc);
    let block_dist = max(receiver_d - blocker_d, 0.0);
    let penumbra_world = (block_dist * light_angle) / max(blocker_d, 1e-3);
    var filter_radius = penumbra_world * shadow_u.shadow_atlas_size * 0.02;
    filter_radius = clamp(filter_radius, texel, texel * 8.0);

    var visibility = 0.0;
    let golden = 2.39996323;
    let rot_angle = dither * 6.28318530718;
    for (var i = 0u; i < 16u; i = i + 1u) {
        let fi = f32(i);
        let angle = fi * golden + rot_angle;
        let r = sqrt(fi / 16.0) * filter_radius;
        let offset = vec2<f32>(cos(angle), sin(angle)) * r;
        visibility += textureSampleCompareLevel(
            t_csm, s_csm, uv + offset, i32(cascade_idx), ndc.z - 0.001,
        );
    }
    return visibility / 16.0;
}

fn get_csm_shadow(world_pos: vec3<f32>, view_z: f32, normal: vec3<f32>, dither: f32) -> f32 {
    let cascade_idx = get_cascade_index(view_z);

    let l_dir = normalize(light_u.dir_direction.xyz);
    let n_dot_l = max(dot(normal, l_dir), 0.0);
    let bias = 0.00025 + (1.0 - n_dot_l) * 0.0012;
    let shadow_pos = world_pos + normal * bias;

    let mat = shadow_u.csm_matrices[cascade_idx];
    let clip = mat * vec4<f32>(shadow_pos, 1.0);
    if (clip.w <= 1.0e-4) {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);

    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0 || ndc.z < 0.0) {
        return 1.0;
    }

    // VSM-only: never fall through to CSM.
    if (light_u.soft_shadows == 2u) {
        let lk = vsm_lookup(world_pos);
        if (lk.valid > 0.5) {
            return vsm_shadow(world_pos, normal, l_dir);
        }
        return 1.0;
    }
    var shadow: f32;
    if (light_u.soft_shadows == 1u) {
        shadow = pcss_csm(world_pos, view_z, normal, 0.016, dither);
    } else {
        shadow = csm_sample(cascade_idx, uv, ndc.z - 0.0002);
    }

    let idx_usize = i32(cascade_idx);
    if (cascade_idx + 1u < 4u && idx_usize >= 0) {
        let next_split = shadow_u.csm_splits[cascade_idx];
        let prev_split = select(0.0, shadow_u.csm_splits[cascade_idx - 1u], cascade_idx > 0u);
        let band = (next_split - prev_split) * 0.1;
        if (view_z > next_split - band) {
            let mat2 = shadow_u.csm_matrices[cascade_idx + 1u];
            let clip2 = mat2 * vec4<f32>(shadow_pos, 1.0);
            let ndc2 = clip2.xyz / clip2.w;
            let uv2 = vec2<f32>(ndc2.x * 0.5 + 0.5, 0.5 - ndc2.y * 0.5);
            if (!(uv2.x < 0.0 || uv2.x > 1.0 || uv2.y < 0.0 || uv2.y > 1.0 || ndc2.z > 1.0 || ndc2.z < 0.0)) {
                let s2 = csm_sample(cascade_idx + 1u, uv2, ndc2.z - 0.0005);
                let t = (view_z - (next_split - band)) / max(band, 1e-4);
                shadow = mix(shadow, s2, clamp(t, 0.0, 1.0));
            }
        }
    }
    return shadow;
}

// ── VSM_SMRT BLOCK (mirrored verbatim from pbr.wgsl — keep in sync) ──────

const VSM_PAGE_VALID: u32 = 0x80000000u;
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
    ray_count: u32,
    samples_per_ray: u32,
    source_angle: f32,
    source_radius: f32,
    max_trace_dist: f32,
    _pad0: vec3<f32>,
    levels: array<VsmLevelGpu, 8>,
    page_tables: array<u32, 2048>,
}

fn vsm_ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

struct VsmLookup {
    valid: f32,
    layer: i32,
    pixel: vec2<i32>,
    pixel_f: vec2<f32>,
    receiver_z: f32,
    z_world_per_ndc: f32,
    texel_world: f32,
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

fn vsm_shadow(world_pos: vec3<f32>, normal: vec3<f32>, light_dir: vec3<f32>) -> f32 {
    let count = min(vsm_info.level_count, 8u);
    if (count == 0u) {
        return 1.0;
    }

    let f = normalize(light_dir);
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
        // Stronger receiver offset on surfaces facing the light (flat-top acne).
        let n_dot_l = max(dot(normalize(normal), f), 0.0);
        let push = lk0.texel_world * mix(0.25, 0.85, n_dot_l);
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
            // Quartic keeps sample 1 at ~1 texel so the umbra attaches to the
            // caster base. Quadratic+min-frac 0.03 started ~20 cm out.
            let dist = max(vsm_info.max_trace_dist * u * u * u * u, lk0.texel_world * 1.5);
            let sp = origin + dir * dist;

            let lk = vsm_lookup(sp);
            if (lk.valid < 0.5) {
                break;
            }

            let slope = sqrt(max(1.0 - abs(dot(normal, f)), 0.0));
            let contact = clamp(1.0 - dist / max(lk.texel_world * 16.0, 1e-4), 0.0, 1.0);
            // Extra bias for near-perpendicular receivers (flat faces → zebra acne).
            let face_on = max(dot(normalize(normal), f), 0.0);
            let bias_ndc = lk.texel_world * (0.35 + 0.55 * slope + 0.9 * face_on)
                * mix(1.0, 0.45, contact) / lk.z_world_per_ndc;
            let spread = clamp(dist * tan_source / max(lk.texel_world, 1e-4), 1.0, 4.0);

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

// ── End VSM block ─────────────────────────────────────────────────────────

// ── Fullscreen triangle ───────────────────────────────────────────────────

struct FsInput {
    @builtin(position) fb_pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FsInput {
    // Triangle covering the viewport: (-1,-1) (3,-1) (-1,3).
    var p = vec2<f32>(
        f32((vi << 1u) & 2u),
        f32(vi & 2u),
    ) * 2.0 - 1.0;
    var out: FsInput;
    out.fb_pos = vec4<f32>(p, 0.0, 1.0);
    return out;
}

// ── Resolve fragment ──────────────────────────────────────────────────────

@fragment
fn fs_main(in: FsInput) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(t_vis_id));
    let px = vec2<i32>(in.fb_pos.xy);

    // UV in sampling space (v=0 at bottom), matching how the forward pass
    // samples SSAO/IBL textures.
    let uv01 = vec2<f32>(in.fb_pos.xy) / vec2<f32>(textureDimensions(t_depth));
    let sample_uv = vec2<f32>(uv01.x, 1.0 - uv01.y);

    let id = textureLoad(t_vis_id, px, 0).r;
    let depth_raw = textureLoad(t_depth, px, 0);

    if (id == SENTINEL || depth_raw >= 1.0) {
        discard; // background or never-rasterized pixel: keep prior color
    }

    let inst_idx = id >> 16u;
    if (inst_idx >= arrayLength(&vg_instances)) {
        discard; // defensive: stale ID buffer vs shrunk instance list
    }
    let inst = vg_instances[inst_idx];

    // One material per asset ⇒ every meshlet of the instance shares it.
    if (inst.meshlet_offset >= arrayLength(&meshlets)) {
        discard;
    }
    let m_idx = meshlets[inst.meshlet_offset].material_index;

    // Fallback material for out-of-range indices instead of nuking pixels.
    var mat: Material;
    mat.base_color = vec4<f32>(0.8, 0.8, 0.8, 1.0);
    mat.emissive = vec4<f32>(0.0);
    mat.metallic = 0.0;
    mat.roughness = 1.0;
    mat.flags = 0u;
    if (m_idx < arrayLength(&vg_materials)) {
        mat = vg_materials[m_idx];
    }

    // Material-level shading only (see header note): no texture branches.
    let albedo = mat.base_color.rgb;
    var n = normalize(textureLoad(t_vis_nd, px, 0).xyz);

    var metallic = mat.metallic;
    var roughness = clamp(mat.roughness, 0.05, 1.0);
    let spec_roughness = apply_geometric_specular_aa(n, roughness);
    let lobe_r = vec3<f32>(
        spec_roughness,
        apply_geometric_specular_aa(n, clamp(mat.layer0_params.x, 0.05, 1.0)),
        apply_geometric_specular_aa(n, clamp(mat.layer1_params.x, 0.05, 1.0)),
    );

    // ── World-position reconstruction ────────────────────────────────────
    let ndc = vec3<f32>(uv01.x * 2.0 - 1.0, (1.0 - uv01.y) * 2.0 - 1.0, depth_raw);
    let view_pos = camera.inv_proj * vec4<f32>(ndc, 1.0);
    if (abs(view_pos.w) < 1.0e-8) {
        return vec4<f32>(0.0, 0.0, 0.0, mat.base_color.a);
    }
    let world_pos = (camera.inv_view * vec4<f32>(view_pos.xyz / view_pos.w, 1.0)).xyz;

    let v = normalize(camera.camera_pos.xyz - world_pos);
    let n_dot_v = max(dot(n, v), 0.0001);
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // --- DEBUG VISUALIZATION OVERLAY (parity with forward pass) ---
    if (shader_debug.debug_viz_mode == 1u) {
        let cluster_x = u32(in.fb_pos.x / 16.0);
        let cluster_y = u32(in.fb_pos.y / 16.0);
        let view_pos_dbg = camera.view * vec4<f32>(world_pos, 1.0);
        let view_z_dbg = max(-view_pos_dbg.z, cluster_info.near);
        let cluster_z = get_depth_slice(view_z_dbg, cluster_info);
        let cluster_idx_flat = cluster_x + cluster_y * cluster_info.clusters_x + cluster_z * cluster_info.clusters_x * cluster_info.clusters_y;
        let color_val = f32(cluster_idx_flat % 128u) / 127.0;
        return vec4<f32>(color_val, color_val * 0.5, 0.0, 1.0);
    } else if (shader_debug.debug_viz_mode == 2u) {
        let view_z = -(camera.view * vec4<f32>(world_pos, 1.0)).z;
        let s = get_csm_shadow(world_pos, view_z, n, shadow_sample_dither(in.fb_pos.xy));
        return vec4<f32>(s, s, s, 1.0);
    }

    var shadow = 1.0;
    var lo = vec3<f32>(0.0);

    // --- Directional Light ---
    {
        let l = normalize(light_u.dir_direction.xyz);
        let n_dot_l = max(dot(n, l), 0.0);
        let view_z = -(camera.view * vec4<f32>(world_pos, 1.0)).z;
        shadow = get_csm_shadow(world_pos, view_z, n, shadow_sample_dither(in.fb_pos.xy));
        if (n_dot_l > 0.0) {
            let radiance = light_u.dir_color.rgb * light_u.dir_color.w;
            lo += eval_substrate_lit(mat, n, v, l, albedo, metallic, lobe_r) * radiance * shadow;
        }
    }

    // --- Point Lights (Clustered Forward Rendering) ---
    let cluster_x = u32(in.fb_pos.x / 16.0);
    let cluster_y = u32(in.fb_pos.y / 16.0);

    let view_pos_cluster = camera.view * vec4<f32>(world_pos, 1.0);
    let view_z_cluster = max(-view_pos_cluster.z, cluster_info.near);
    let cluster_z = get_depth_slice(view_z_cluster, cluster_info);

    if (cluster_x < cluster_info.clusters_x && cluster_y < cluster_info.clusters_y && cluster_z < cluster_info.depth_slices) {
        let cluster_idx = cluster_x + cluster_y * cluster_info.clusters_x + cluster_z * cluster_info.clusters_x * cluster_info.clusters_y;

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

            let l = normalize(p_pos - world_pos);
            let dist = length(p_pos - world_pos);

            let attenuation = 1.0 / (dist * dist + 1.0) * (1.0 - smoothstep(0.0, p_radius, dist));
            let radiance = p_color * p_intensity;

            let p_shadow = get_point_light_shadow(light, world_pos, n);
            let n_dot_l = max(dot(n, l), 0.0);
            if (n_dot_l > 0.0) {
                lo += eval_substrate_lit(mat, n, v, l, albedo, metallic, lobe_r) * radiance * attenuation * p_shadow;
            }
        }
    }

    // --- Ambient IBL ---
    let kS_ibl = fresnel_schlick_roughness(n_dot_v, f0, spec_roughness);
    let kD_ibl = (vec3<f32>(1.0) - kS_ibl) * (1.0 - metallic);

    var irradiance = vec3<f32>(0.0);
    var prefiltered_color = vec3<f32>(0.0);
    var env_brdf = vec2<f32>(0.0, 0.0);
    let r = reflect(-v, n);
    if (shader_debug.ibl_enabled != 0u) {
        irradiance = textureSample(t_irradiance, s_ibl, n).rgb;
        prefiltered_color = textureSampleLevel(t_prefiltered, s_ibl, r, spec_roughness * 4.0).rgb;
        env_brdf = textureSample(t_brdf_lut, s_ibl, vec2<f32>(n_dot_v, spec_roughness)).rg;
    }
    let diffuse_ibl = irradiance * albedo;
    let specular_ibl = prefiltered_color * (kS_ibl * env_brdf.x + env_brdf.y);

    let ao = textureSample(t_ssao, s_ssao, sample_uv).r;

    let ambient_factor = light_u.ambient.w;

    // Substrate layer IBL (weighted diffuse-ish contribution), mirrors pbr.wgsl.
    var substrate_ibl = vec3<f32>(0.0);
    let w0 = mat.layer0_params.y;
    let w1 = mat.layer1_params.y;
    if (w0 > 1.0e-4 && shader_debug.ibl_enabled != 0u) {
        let a0 = mat.layer0_color.xyz;
        let m0 = mat.layer0_color.w;
        let r0 = clamp(mat.layer0_params.x, 0.05, 1.0);
        let f0_0 = mix(vec3<f32>(0.04), a0, m0);
        let ks0 = fresnel_schlick_roughness(n_dot_v, f0_0, r0);
        let kd0 = (vec3<f32>(1.0) - ks0) * (1.0 - m0);
        let pre0 = textureSampleLevel(t_prefiltered, s_ibl, r, r0 * 4.0).rgb;
        let brdf0 = textureSample(t_brdf_lut, s_ibl, vec2<f32>(n_dot_v, r0)).rg;
        substrate_ibl = substrate_ibl + w0 * (kd0 * irradiance * a0 + pre0 * (ks0 * brdf0.x + brdf0.y));
    }
    if (w1 > 1.0e-4 && shader_debug.ibl_enabled != 0u) {
        let a1 = mat.layer1_color.xyz;
        let m1 = mat.layer1_color.w;
        let r1 = clamp(mat.layer1_params.x, 0.05, 1.0);
        let f0_1 = mix(vec3<f32>(0.04), a1, m1);
        let ks1 = fresnel_schlick_roughness(n_dot_v, f0_1, r1);
        let kd1 = (vec3<f32>(1.0) - ks1) * (1.0 - m1);
        let pre1 = textureSampleLevel(t_prefiltered, s_ibl, r, r1 * 4.0).rgb;
        let brdf1 = textureSample(t_brdf_lut, s_ibl, vec2<f32>(n_dot_v, r1)).rg;
        substrate_ibl = substrate_ibl + w1 * (kd1 * irradiance * a1 + pre1 * (ks1 * brdf1.x + brdf1.y));
    }
    let w_base = max(1.0 - w0 - w1, 0.0);

    let ambient = (w_base * (kD_ibl * diffuse_ibl + specular_ibl) + substrate_ibl) * ambient_factor * ao + light_u.ambient.xyz * albedo * ao;
    let color = ambient + lo + mat.emissive.rgb;
    return vec4<f32>(sanitize_hdr(color), mat.base_color.a);
}

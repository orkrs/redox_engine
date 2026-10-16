// SDF cone-traced shadows — GPU companion to the CPU reference
// `SdfShadowScene::trace_cone` in src/sdf/mod.rs.
//
// Bindings 26..28 match `pbr.wgsl`. Multi-brick page table lives in
// `SdfSceneGpu.pages` (up to 8 bricks packed into the 128³ atlas).
//
// Sign convention matches the CPU baker: NEGATIVE inside solids, positive
// outside, meters everywhere.

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
};

@group(0) @binding(26) var<uniform> sdf_scene: SdfSceneGpu;
@group(0) @binding(27) var t_sdf_bricks: texture_3d<f32>;
@group(0) @binding(28) var s_sdf_bricks: sampler;

const SDF_CONE_APERTURE: f32 = 0.3;
const MAX_CONE_STEPS: u32 = 128u;
const SDF_ATLAS_EXTENT: f32 = 128.0;

fn sdf_load_atlas(coord: vec3<i32>) -> f32 {
    let c = clamp(coord, vec3<i32>(0), vec3<i32>(i32(SDF_ATLAS_EXTENT) - 1));
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
            best = min(best, d_box);
        }
    }
    return best;
}

fn sdf_trace_shadow(world_pos: vec3<f32>, light_dir: vec3<f32>, normal: vec3<f32>) -> f32 {
    if (sdf_scene.atlas_dims.x == 0u) {
        return 1.0;
    }
    let texel = sdf_scene.world_bounds_max_texel.w;
    let bias = 2.0 * texel;
    let origin = world_pos + normal * bias;
    let dir = normalize(light_dir);
    let max_dist = length(sdf_scene.world_bounds_max_texel.xyz - sdf_scene.world_bounds_min.xyz);
    let half_tan = tan(0.5 * SDF_CONE_APERTURE);
    let t_min = max(bias, 0.5 * texel);

    var visibility = 1.0;
    var t = t_min;

    for (var i = 0u; i < MAX_CONE_STEPS; i = i + 1u) {
        if (t >= max_dist || visibility < 1.0e-3) {
            break;
        }
        let p = origin + dir * t;
        let sd = sdf_sample_world(p);
        let r = half_tan * t;

        var occ: f32;
        if (r > 1.0e-6) {
            occ = clamp(1.0 - sd / r, 0.0, 1.0);
        } else {
            occ = select(0.0, 1.0, sd <= 0.0);
        }

        var dt: f32;
        if (sd <= 0.0) {
            dt = t_min;
        } else {
            dt = clamp(abs(sd), t_min, max(r, t_min));
        }
        let cov = min(1.0, dt / max(r, t_min));

        visibility = visibility * (1.0 - occ * cov);
        t = t + dt;
    }

    return clamp(visibility, 0.0, 1.0);
}

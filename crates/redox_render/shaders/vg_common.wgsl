// Shared culling helpers of the VG compute shaders (`vg_cull`, `vg_dag_traverse`).
//
// Concatenated in front of the shader body by `crate::shader`. The including shader declares the
// bindings used here: `cull_params` (uniform CullParams), `t_hiz` (texture_2d<f32>) and `s_hiz` (sampler).

struct CullParams {
    planes: array<vec4<f32>, 6>,
    camera_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    hiz_size: vec2<f32>,
    hiz_mip_count: u32,
    hiz_enabled: u32,
    cull_mode: u32, // 0 = phase 1, 1 = phase 2
    reject_capacity: u32,
    _pad: vec2<u32>,
};

fn test_sphere(center: vec3<f32>, radius: f32) -> bool {
    for (var i = 0; i < 6; i++) {
        let p = cull_params.planes[i];
        let d = dot(p.xyz, center) + p.w;
        if (d < -radius) {
            return false;
        }
    }
    return true;
}

fn max_scale(m: mat4x4<f32>) -> f32 {
    return max(length(m[0].xyz), max(length(m[1].xyz), length(m[2].xyz)));
}

// Mip whose texels are at least as large as the rect (`rect_size` in mip-0 texels), so that the
// 2x2 samples at the rect corners cover all of it.
fn hiz_mip_for_rect(rect_size: vec2<f32>, tex_size: vec2<f32>) -> u32 {
    let max_side = max(rect_size.x, rect_size.y);
    let level = ceil(log2(max(max_side, 1.0)));
    return u32(clamp(level, 0.0, f32(cull_params.hiz_mip_count - 1u)));
}

// True when the sphere may be visible: its nearest point is not behind the farthest depth of the
// Hi-Z texels under its screen rect. Conservative: spheres reaching the camera plane, and spheres
// whose rect needs a coarser mip than the pyramid has, count as visible.
//
// The rect is the sphere's projection bounded by `radius / (w - radius)` times the projection
// scales (the lengths of the first two rows of `view_proj`, whose view part is a rotation), and
// Hi-Z rows go down the screen (texture space), NDC y up.
fn test_hiz_occlusion(center: vec3<f32>, radius: f32, view_proj: mat4x4<f32>) -> bool {
    if (cull_params.hiz_enabled == 0u) {
        return true;
    }

    let clip = view_proj * vec4<f32>(center, 1.0);
    let near_w = clip.w - radius;
    if (near_w <= 1.0e-3) {
        return true;
    }
    let ndc = clip.xy / clip.w;
    let scale = vec2<f32>(
        length(vec3<f32>(view_proj[0].x, view_proj[1].x, view_proj[2].x)),
        length(vec3<f32>(view_proj[0].y, view_proj[1].y, view_proj[2].y)),
    );
    let r_uv = scale * (0.5 * radius / near_w);
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let uv_min = clamp(uv - r_uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let uv_max = clamp(uv + r_uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let rect_size = (uv_max - uv_min) * cull_params.hiz_size;
    let max_side = max(rect_size.x, rect_size.y);
    if (max_side > exp2(f32(cull_params.hiz_mip_count - 1u))) {
        return true;
    }
    let mip = hiz_mip_for_rect(rect_size, cull_params.hiz_size);

    var max_hiz = 0.0;
    for (var sy = 0u; sy < 2u; sy++) {
        for (var sx = 0u; sx < 2u; sx++) {
            let sample_uv = vec2<f32>(
                select(uv_min.x, uv_max.x, sx == 1u),
                select(uv_min.y, uv_max.y, sy == 1u),
            );
            max_hiz = max(max_hiz, textureSampleLevel(t_hiz, s_hiz, sample_uv, f32(mip)).x);
        }
    }

    let to_cam = normalize(cull_params.camera_pos.xyz - center);
    let nearest = center + to_cam * radius;
    let nearest_clip = view_proj * vec4<f32>(nearest, 1.0);
    let nearest_ndc_z = nearest_clip.z / nearest_clip.w;
    return nearest_ndc_z <= max_hiz + 0.002;
}

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

fn hiz_mip_for_rect(rect_size: vec2<f32>, tex_size: vec2<f32>) -> u32 {
    let max_side = max(rect_size.x, rect_size.y);
    let level = ceil(log2(max(max_side, 1.0) / max(tex_size.x, 1.0)));
    return u32(clamp(level, 0.0, f32(cull_params.hiz_mip_count - 1u)));
}

fn test_hiz_occlusion(center: vec3<f32>, radius: f32, view_proj: mat4x4<f32>) -> bool {
    if (cull_params.hiz_enabled == 0u) {
        return true;
    }

    let clip = view_proj * vec4<f32>(center, 1.0);
    if (clip.w <= 0.0) {
        return false;
    }
    let ndc = clip.xyz / clip.w;
    if (abs(ndc.x) > 1.0 + radius || abs(ndc.y) > 1.0 + radius) {
        return false;
    }

    let uv = ndc.xy * 0.5 + 0.5;
    let px_radius = radius * cull_params.hiz_size.y * 0.5 / clip.w;
    let rect_min = uv * cull_params.hiz_size - vec2<f32>(px_radius);
    let rect_max = uv * cull_params.hiz_size + vec2<f32>(px_radius);
    let rect_size = max(rect_max - rect_min, vec2<f32>(1.0));
    let mip = hiz_mip_for_rect(rect_size, cull_params.hiz_size);

    var max_hiz = 0.0;
    for (var sy = 0u; sy < 2u; sy++) {
        for (var sx = 0u; sx < 2u; sx++) {
            let su = (f32(sx) + 0.5) / 2.0;
            let sv = (f32(sy) + 0.5) / 2.0;
            let sample_uv = mix(rect_min, rect_max, vec2<f32>(su, sv)) / cull_params.hiz_size;
            max_hiz = max(max_hiz, textureSampleLevel(t_hiz, s_hiz, sample_uv, f32(mip)).x);
        }
    }

    let to_cam = normalize(cull_params.camera_pos.xyz - center);
    let nearest = center + to_cam * radius;
    let nearest_clip = view_proj * vec4<f32>(nearest, 1.0);
    let nearest_ndc_z = nearest_clip.z / nearest_clip.w;
    return nearest_ndc_z <= max_hiz + 0.002;
}

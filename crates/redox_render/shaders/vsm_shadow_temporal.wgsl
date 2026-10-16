// Temporal accumulation for 1-spp VSM shadow trace.

struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

struct VsmTemporalUniform {
    blend_alpha: f32,
    reset: u32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var t_raw: texture_2d<f32>;
@group(0) @binding(1) var t_history: texture_2d<f32>;
@group(0) @binding(2) var t_normal_depth: texture_2d<f32>;
@group(0) @binding(3) var<uniform> camera: Camera;
@group(0) @binding(4) var s_linear: sampler;
@group(0) @binding(5) var<uniform> vsm_temp: VsmTemporalUniform;

const FAR: f32 = 1000.0;

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
    return (camera.inv_view * vec4<f32>(view_pos_from_uv_depth(uv, linear_depth), 1.0)).xyz;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) f32 {
    let dims = vec2<f32>(textureDimensions(t_normal_depth));
    let px = vec2<i32>(clamp(in.uv * dims, vec2<f32>(0.0), dims - 1.0));
    let nd = textureLoad(t_normal_depth, px, 0);
    let curr = textureSample(t_raw, s_linear, in.uv).r;

    if (nd.a >= 0.999 || nd.a <= 1e-5 || vsm_temp.reset != 0u) {
        return curr;
    }

    let world_pos = world_pos_from_uv_depth(in.uv, nd.a);
    let prev_clip = camera.prev_view_proj * vec4<f32>(world_pos, 1.0);
    if (prev_clip.w <= 1e-4) {
        return curr;
    }
    let prev_ndc = prev_clip.xy / prev_clip.w;
    let prev_uv = vec2<f32>(prev_ndc.x * 0.5 + 0.5, 0.5 - prev_ndc.y * 0.5);
    if (prev_uv.x < 0.0 || prev_uv.x > 1.0 || prev_uv.y < 0.0 || prev_uv.y > 1.0) {
        return curr;
    }

    // Depth disocclusion — reject stale history on motion / new geometry.
    // World-space threshold scaled with depth to keep far-field stable.
    let prev_px = vec2<i32>(clamp(prev_uv * dims, vec2<f32>(0.0), dims - 1.0));
    let prev_nd = textureLoad(t_normal_depth, prev_px, 0);
    let depth_diff = abs(nd.a - prev_nd.a) * FAR;
    let depth_thresh = 0.15 + nd.a * 0.30;
    if (depth_diff > depth_thresh) {
        return curr;
    }

    let hist = textureSample(t_history, s_linear, prev_uv).r;

    // Neighborhood AABB clamp on history (reduces smear trails).
    // Variance clamp (mean ± 1.25 sigma) would be more permissive on edges
    // but AABB is sufficient for binary R16Float visibility.
    let texel = 1.0 / dims;
    var min_s = curr;
    var max_s = curr;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let s = textureSample(t_raw, s_linear, in.uv + vec2<f32>(f32(x), f32(y)) * texel).r;
            min_s = min(min_s, s);
            max_s = max(max_s, s);
        }
    }
    let clamped_hist = clamp(hist, min_s, max_s);

    let alpha = clamp(vsm_temp.blend_alpha, 0.05, 0.20);
    return mix(clamped_hist, curr, alpha);
}

// Ground-Truth AO (slice integration) + bent normals (world space).
// Output: RGB = bent normal, A = scalar visibility [0,1].

struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var t_normal_depth: texture_2d<f32>;
@group(0) @binding(1) var t_noise: texture_2d<f32>;
@group(0) @binding(2) var s_gtao: sampler;
@group(0) @binding(3) var<uniform> camera: Camera;

const FAR: f32 = 1000.0;
const SLICES: u32 = 3u;
const STEPS: u32 = 4u;
const RADIUS_M: f32 = 0.55;

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

fn project_view_to_uv(view_pos: vec3<f32>) -> vec2<f32> {
    let clip = camera.proj * vec4<f32>(view_pos, 1.0);
    let ndc = clip.xy / max(abs(clip.w), 1e-5);
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(t_normal_depth));
    let px = vec2<i32>(clamp(in.uv * dims, vec2<f32>(0.0), dims - 1.0));
    let nd = textureLoad(t_normal_depth, px, 0);
    let n_world = normalize(nd.rgb);
    let depth = nd.a;
    if (depth >= 0.999 || depth <= 1e-5) {
        return vec4<f32>(n_world, 1.0);
    }

    let view_pos = view_pos_from_uv_depth(in.uv, depth);
    let n_view = normalize((camera.view * vec4<f32>(n_world, 0.0)).xyz);

    let noise_px = vec2<i32>(i32(in.clip_pos.x) & 3, i32(in.clip_pos.y) & 3);
    let noise = textureLoad(t_noise, noise_px, 0).x;

    let texel = 1.0 / dims;
    let radius_uv = clamp(RADIUS_M / max(depth * FAR, 0.5), texel.x * 2.0, 0.08);

    // GTAO horizon integration (XeGTAO / Filament style) + bent normal.
    // Radius in UV: clamp to avoid sampling beyond half-res blur kernel.
    var vis_sum = 0.0;
    var bent_view = vec3<f32>(0.0);
    var bent_w = 0.0;

    for (var s = 0u; s < SLICES; s++) {
        let slice_a = (f32(s) + noise) * 6.28318530718 / f32(SLICES);
        let dir2 = vec2<f32>(cos(slice_a), sin(slice_a));
        var h_cos_pos = -1.0;
        var h_cos_neg = -1.0;
        var dir_accum = vec3<f32>(0.0);
        var dir_w = 0.0;

        for (var st = 1u; st <= STEPS; st++) {
            let t = f32(st) / f32(STEPS);
            let off = dir2 * radius_uv * t;
            let suv = in.uv + off;
            if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) {
                continue;
            }
            let spx = vec2<i32>(clamp(suv * dims, vec2<f32>(0.0), dims - 1.0));
            let sd = textureLoad(t_normal_depth, spx, 0).a;
            if (sd >= 0.999) {
                continue;
            }
            let s_view = view_pos_from_uv_depth(suv, sd);
            let v = s_view - view_pos;
            let proj = dot(v, n_view);
            let tan_v = v - n_view * proj;
            let len_t = length(tan_v);
            if (len_t < 1e-5) {
                continue;
            }
            let slice_t = vec3<f32>(dir2.x, dir2.y, 0.0);
            let sign_d = sign(dot(normalize(tan_v), slice_t));
            let h = proj / max(len_t, 1e-4);
            let h_cos = h / sqrt(h * h + 1.0);
            // Horizon angle tracking per side.
            if (sign_d > 0.0) {
                h_cos_pos = max(h_cos_pos, h_cos);
            } else {
                h_cos_neg = max(h_cos_neg, h_cos);
            }
            // Bent direction: weight by UNoccluded direction (opposite to occluder).
            // tan_v points TOWARD the occluder; negate to tilt bent normal into
            // the open hemisphere. Horizon factor (1-h_cos) is larger for blocked samples.
            let falloff = 1.0 - t * 0.5;
            let w = max(0.0, 1.0 - h_cos) * falloff;
            dir_accum = dir_accum + normalize(tan_v) * w * -1.0;
            dir_w += w;
        }

        // Visibility per side: map h_cos∈[-1,1] to visibility. When no hit (h_cos==-1)
        // treat as fully visible (1.0) instead of 0.25+0.75*(-1)= -0.5.
        let v_pos = select(1.0, 0.25 + 0.75 * clamp(h_cos_pos, -1.0, 1.0), h_cos_pos > -0.99);
        let v_neg = select(1.0, 0.25 + 0.75 * clamp(h_cos_neg, -1.0, 1.0), h_cos_neg > -0.99);
        vis_sum += v_pos + v_neg;
        if (dir_w > 0.0) {
            bent_view = bent_view + dir_accum / dir_w;
            bent_w += 1.0;
        }
    }

    let ao = clamp(vis_sum / max(f32(SLICES) * 2.0, 1.0), 0.05, 1.0);
    var bent_world = n_world;
    if (bent_w > 0.0) {
        // Blend accumulated unoccluded directions with geometric view normal
        // to avoid flipping. Weights tuned so flat surfaces keep n_view.
        let bv = normalize(bent_view / bent_w * 0.6 + n_view * 1.4);
        bent_world = normalize((camera.inv_view * vec4<f32>(bv, 0.0)).xyz);
    }
    return vec4<f32>(bent_world, ao);
}


struct TaaUniform {
    output_size : vec2<f32>,
    input_size  : vec2<f32>,
    blend_alpha : f32,
    reset       : u32,
    jitter_uv   : vec2<f32>,
};

@group(0) @binding(0) var t_current    : texture_2d<f32>;
@group(0) @binding(1) var t_velocity   : texture_2d<f32>; // xy = UV delta, z = depth
@group(0) @binding(2) var t_history    : texture_2d<f32>;
@group(0) @binding(3) var s_linear     : sampler;
@group(0) @binding(4) var<uniform>     taa_u : TaaUniform;
@group(0) @binding(5) var t_reactive   : texture_2d<f32>;
@group(0) @binding(6) var t_prev_depth : texture_depth_2d; // previous frame depth

struct VsOut {
    @builtin(position) clip_pos : vec4<f32>,
    @location(0)       uv       : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    out.uv       = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

fn rgb_to_ycocg(c: vec3<f32>) -> vec3<f32> {
    let y  = 0.25 * c.r + 0.5 * c.g + 0.25 * c.b;
    let co = 0.5 * c.r - 0.5 * c.b;
    let cg = -0.25 * c.r + 0.5 * c.g - 0.25 * c.b;
    return vec3<f32>(y, co, cg);
}

fn ycocg_to_rgb(ycc: vec3<f32>) -> vec3<f32> {
    let y  = ycc.x;
    let co = ycc.y;
    let cg = ycc.z;
    return max(vec3<f32>(y + co - cg, y + cg, y - co - cg), vec3<f32>(0.0));
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let l = 0.25 * c.r + 0.5 * c.g + 0.25 * c.b;
    return c / (1.0 + max(l, 0.0));
}

fn inv_tonemap(c: vec3<f32>) -> vec3<f32> {
    let l = 0.25 * c.r + 0.5 * c.g + 0.25 * c.b;
    return c / max(1.0 - min(l, 0.999), 1e-4);
}

fn finite_hdr(c: vec3<f32>) -> bool {
    return !(any(c != c) || any(abs(c) > vec3<f32>(65504.0)));
}

fn bh_weight(d: vec2<f32>) -> f32 {
    return exp(-2.29 * dot(d, d));
}

fn sample_history_catmull_rom(uv: vec2<f32>) -> vec3<f32> {
    let size = taa_u.output_size;
    let texel = 1.0 / size;
    let sample_pos = uv * size;
    let tc = floor(sample_pos - 0.5) + 0.5;
    let f = sample_pos - tc;
    let f2 = f * f;
    let f3 = f2 * f;

    let w0 = f2 - 0.5 * (f3 + f);
    let w1 = 1.5 * f3 - 2.5 * f2 + 1.0;
    let w3 = 0.5 * (f3 - f2);
    let w2 = 1.0 - w0 - w1 - w3;

    let w12 = w1 + w2;
    let tc12 = (tc + w2 / max(w12, vec2<f32>(1e-5))) * texel;
    let tc0 = (tc - 1.0) * texel;
    let tc3 = (tc + 2.0) * texel;

    var result = vec3<f32>(0.0);
    var wsum = 0.0;

    let c_top = textureSampleLevel(t_history, s_linear, vec2<f32>(tc12.x, tc0.y), 0.0).rgb;
    let w_top = w12.x * w0.y;
    if (finite_hdr(c_top)) {
        result += c_top * w_top;
        wsum += w_top;
    }
    let c_left = textureSampleLevel(t_history, s_linear, vec2<f32>(tc0.x, tc12.y), 0.0).rgb;
    let w_left = w0.x * w12.y;
    if (finite_hdr(c_left)) {
        result += c_left * w_left;
        wsum += w_left;
    }
    let c_mid = textureSampleLevel(t_history, s_linear, vec2<f32>(tc12.x, tc12.y), 0.0).rgb;
    let w_mid = w12.x * w12.y;
    if (finite_hdr(c_mid)) {
        result += c_mid * w_mid;
        wsum += w_mid;
    }
    let c_right = textureSampleLevel(t_history, s_linear, vec2<f32>(tc3.x, tc12.y), 0.0).rgb;
    let w_right = w3.x * w12.y;
    if (finite_hdr(c_right)) {
        result += c_right * w_right;
        wsum += w_right;
    }
    let c_bot = textureSampleLevel(t_history, s_linear, vec2<f32>(tc12.x, tc3.y), 0.0).rgb;
    let w_bot = w12.x * w3.y;
    if (finite_hdr(c_bot)) {
        result += c_bot * w_bot;
        wsum += w_bot;
    }
    let filtered = result / max(wsum, 1e-5);

    let c00 = textureSampleLevel(t_history, s_linear, (tc + vec2<f32>(0.0, 0.0)) * texel, 0.0).rgb;
    let c10 = textureSampleLevel(t_history, s_linear, (tc + vec2<f32>(1.0, 0.0)) * texel, 0.0).rgb;
    let c01 = textureSampleLevel(t_history, s_linear, (tc + vec2<f32>(0.0, 1.0)) * texel, 0.0).rgb;
    let c11 = textureSampleLevel(t_history, s_linear, (tc + vec2<f32>(1.0, 1.0)) * texel, 0.0).rgb;
    let mn = min(min(min(c00, c10), c01), c11);
    let mx = max(max(max(c00, c10), c01), c11);
    return clamp(filtered, mn, mx);
}

fn clip_aabb(mn: vec3<f32>, mx: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let center = 0.5 * (mx + mn);
    let extents = max(0.5 * (mx - mn), vec3<f32>(1e-4));
    let offset = p - center;
    let v_unit = offset / extents;
    let ma = max(abs(v_unit.x), max(abs(v_unit.y), abs(v_unit.z)));
    if (ma > 1.0) {
        return center + offset / ma;
    }
    return p;
}

// TAA-STAR hybrid: longest MV on motion, closest depth at silhouettes.
fn dilated_velocity(center_px: vec2<i32>, max_px: vec2<i32>) -> vec2<f32> {
    var best_d = 2.0;
    var closest_v = textureLoad(t_velocity, center_px, 0).xy;
    var longest_v = closest_v;
    var longest_len = dot(longest_v, longest_v);
    var depth_min = 2.0;
    var depth_max = 0.0;

    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let coord = clamp(center_px + vec2<i32>(dx, dy), vec2<i32>(0), max_px);
            let s = textureLoad(t_velocity, coord, 0);
            depth_min = min(depth_min, s.z);
            depth_max = max(depth_max, s.z);
            if (s.z < best_d) {
                best_d = s.z;
                closest_v = s.xy;
            }
            let len = dot(s.xy, s.xy);
            if (len > longest_len) {
                longest_len = len;
                longest_v = s.xy;
            }
        }
    }
    let depth_edge = depth_max - depth_min;
    if (depth_edge > 0.002 && longest_len > 1e-10) {
        return longest_v;
    }
    return closest_v;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let max_px = vec2<i32>(taa_u.input_size) - vec2<i32>(1);
    let px = vec2<i32>(clamp(
        in.uv * taa_u.input_size,
        vec2<f32>(0.0),
        taa_u.input_size - vec2<f32>(1.0),
    ));

    let jitter_px = taa_u.jitter_uv * taa_u.input_size;
    var m1 = vec3<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    var cmc = vec3<f32>(0.0);

    var cur_sum = vec3<f32>(0.0);
    var cur_wsum = 0.0;
    var center_w = 0.0;

    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let coord = clamp(px + vec2<i32>(dx, dy), vec2<i32>(0), max_px);
            let raw = textureLoad(t_current, coord, 0).rgb;
            let ycc = rgb_to_ycocg(tonemap(raw));
            m1 += ycc;
            m2 += ycc * ycc;
            if (dx == 0 && dy == 0) {
                cmc = ycc;
            }
            let d = vec2<f32>(f32(dx), f32(dy)) - jitter_px;
            let w = bh_weight(d);
            cur_sum += tonemap(raw) * w;
            cur_wsum += w;
            if (dx == 0 && dy == 0) {
                center_w = w;
            }
        }
    }

    let mu = m1 * (1.0 / 9.0);
    let sigma = sqrt(max(m2 * (1.0 / 9.0) - mu * mu, vec3<f32>(0.0)));

    let vel_conf = 1.0 - clamp(length(dilated_velocity(px, max_px) * taa_u.output_size) / 24.0, 0.0, 1.0);
    let gamma = mix(0.85, 1.45, vel_conf * vel_conf);
    var cmin = mu - gamma * sigma;
    var cmax = mu + gamma * sigma;
    let chroma_extent = 0.125 * max(cmax.x - cmin.x, sigma.x);
    cmin = vec3<f32>(cmin.x, cmc.y - chroma_extent, cmc.z - chroma_extent);
    cmax = vec3<f32>(cmax.x, cmc.y + chroma_extent, cmc.z + chroma_extent);

    let raw_vel = dilated_velocity(px, max_px);
    let velocity = raw_vel - taa_u.jitter_uv;
    let reprojected_uv = in.uv - velocity;
    let in_bounds = all(reprojected_uv >= vec2<f32>(0.0)) && all(reprojected_uv <= vec2<f32>(1.0));

    let vel_len = length(velocity * taa_u.output_size);
    let still = 1.0 - clamp(vel_len / 1.25, 0.0, 1.0);

    let current_tm = cur_sum / max(cur_wsum, 1e-5);
    let current_ycc = rgb_to_ycocg(current_tm);

    var history_ycc = rgb_to_ycocg(tonemap(sample_history_catmull_rom(reprojected_uv)));

    let lum_diff = abs(current_ycc.x - history_ycc.x)
        / max(current_ycc.x, max(history_ycc.x, 0.2));
    let agree = 1.0 - clamp(lum_diff / 0.22, 0.0, 1.0);
    let is_edge = clamp(sigma.x / 0.05, 0.0, 1.0);
    let lock = still * still * is_edge * agree;
    let ghost = (1.0 - is_edge) * clamp(lum_diff / 0.10, 0.0, 1.0);

    let clipped = clip_aabb(cmin, cmax, history_ycc);
    history_ycc = mix(clipped, history_ycc, lock);
    let history_tm = ycocg_to_rgb(history_ycc);

    // Intel / TAA-STAR depth threshold disocclusion reject.
    let curr_depth = textureLoad(t_velocity, px, 0).z;
    let prev_px = vec2<i32>(reprojected_uv * vec2<f32>(textureDimensions(t_prev_depth)));
    let prev_depth = textureLoad(t_prev_depth, prev_px, 0);
    let depth_ok = step(curr_depth, prev_depth + 0.001);

    var k_move = mix(1.0 - taa_u.blend_alpha * 2.0, 1.0 - taa_u.blend_alpha, agree * agree);
    k_move *= 1.0 - clamp(vel_len / 12.0, 0.0, 1.0) * 0.55;
    k_move *= 1.0 - ghost;
    // Cap history weight on static edges — 0.97 was too soft for MSAA+TAA hybrids.
    var k_feedback = mix(k_move, 0.80, lock);

    let phase_w = center_w / max(cur_wsum, 1e-5);
    k_feedback *= mix(0.85, 1.0, phase_w);

    if (!in_bounds || taa_u.reset != 0u || depth_ok < 0.5) {
        k_feedback = 0.0;
    }

    let reactive = textureLoad(
        t_reactive,
        vec2<i32>(in.uv * vec2<f32>(textureDimensions(t_reactive))),
        0,
    ).r;
    k_feedback *= 1.0 - reactive;

    let resolved_tm = mix(current_tm, history_tm, clamp(k_feedback, 0.0, 1.0));
    return vec4<f32>(inv_tonemap(resolved_tm), 1.0);
}

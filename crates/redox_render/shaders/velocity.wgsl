
struct VelocityUniform {
    inv_curr_vp : mat4x4<f32>,
    prev_vp     : mat4x4<f32>,
    screen_size : vec2<f32>,
    jitter_uv   : vec2<f32>,
};

@group(0) @binding(0) var t_depth  : texture_depth_2d;
@group(0) @binding(1) var<uniform> vel_u : VelocityUniform;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let px    = vec2<i32>(i32(in.clip_pos.x), i32(in.clip_pos.y));
    let depth = textureLoad(t_depth, px, 0);

    // Sky/background: write the same jitter-induced UV delta as static geometry
    // so TAA's `vel - jitter_uv` cancels to 0. Returning 0 here made silhouettes
    // mix object (≈jitter) with sky (0) and crawl after the jitter subtract.
    if (depth >= 0.9999) {
        return vec4<f32>(vel_u.jitter_uv, depth, 1.0);
    }

    let ndc_x = in.uv.x * 2.0 - 1.0;
    let ndc_y = 1.0 - 2.0 * in.uv.y;

    let ndc_h    = vec4<f32>(ndc_x, ndc_y, depth, 1.0);
    let world_h  = vel_u.inv_curr_vp * ndc_h;
    let world_pos = world_h.xyz / world_h.w;

    let prev_clip = vel_u.prev_vp * vec4<f32>(world_pos, 1.0);
    let prev_ndc  = prev_clip.xy / prev_clip.w;
    let prev_uv = vec2<f32>(prev_ndc.x * 0.5 + 0.5,
                             0.5 - prev_ndc.y * 0.5);

    let vel = in.uv - prev_uv;
    return vec4<f32>(vel, depth, 1.0);
}

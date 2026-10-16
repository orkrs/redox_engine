// Skybox background pass.
//
// Draws a fullscreen triangle that samples the environment cubemap along the
// view ray reconstructed from the inverse camera matrices. Runs FIRST in the
// forward HDR pass (depth is still cleared to 1.0) and writes depth = 1.0 so
// every opaque fragment drawn afterwards wins the depth test.
//
// The Camera layout MUST match `CameraUniform` in src/camera.rs
// (7 mat4 + 1 vec4 = 464 bytes, uniform address alignment).

struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var t_env: texture_cube<f32>;
@group(0) @binding(1) var s_env: sampler;
@group(0) @binding(2) var<uniform> camera: Camera;

struct SkyboxVertexOutput {
    // z = w so the fragment depth lands exactly on the far plane (1.0).
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> SkyboxVertexOutput {
    // Fullscreen triangle trick covering NDC [-1, 1].
    let x = f32((idx << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(idx & 2u) * 2.0 - 1.0;

    var out: SkyboxVertexOutput;
    // z = w puts the fragment depth exactly on the far plane (1.0).
    out.clip_pos = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: SkyboxVertexOutput) -> @location(0) vec4<f32> {
    // Unproject the far-plane point to view space, then rotate the direction
    // into world space (inv_view applied with w = 0 keeps it a direction).
    let far_view = camera.inv_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir_view = normalize(far_view.xyz / far_view.w);
    let dir_world = normalize((camera.inv_view * vec4<f32>(dir_view, 0.0)).xyz);

    // textureSampleLevel: no screen-space derivatives wanted on a cube lookup.
    let color = textureSampleLevel(t_env, s_env, dir_world, 0.0).rgb;
    return vec4<f32>(color, 1.0);
}

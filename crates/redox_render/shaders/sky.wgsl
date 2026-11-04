// Procedural sky pass (SkyMode::Procedural). Replaces the skybox pass in the forward HDR
// pass: fullscreen triangle at depth 1.0, sky-view LUT lookup plus the sun disc.
// Rust prepends `sky_common.wgsl` and `sky_lookup.wgsl`. Bindings match the explicit
// bind group layout in src/sky.rs (SkyAtmosphere::pass_layout).
//
// The Camera layout MUST match `CameraUniform` in src/camera.rs.

struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> atm: Atmosphere;
@group(0) @binding(1) var t_transmittance: texture_2d<f32>;
@group(0) @binding(2) var s_lut: sampler;
@group(0) @binding(3) var t_multiscatter: texture_2d<f32>;
@group(0) @binding(4) var t_sky_view: texture_2d<f32>;
@group(0) @binding(5) var s_sky: sampler;
@group(0) @binding(6) var<uniform> camera: Camera;

// Cap on the disc radiance so it stays finite in Rgba16Float. Sun-like illuminance over the
// disc's solid angle is far above 65504, and the cap still keeps the disc the brightest part.
const SUN_DISK_MAX: f32 = 60000.0;

struct SkyVertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> SkyVertexOutput {
    let x = f32((idx << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(idx & 2u) * 2.0 - 1.0;
    var out: SkyVertexOutput;
    out.clip_pos = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: SkyVertexOutput) -> @location(0) vec4<f32> {
    let far_view = camera.inv_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir_view = normalize(far_view.xyz / far_view.w);
    let d = normalize((camera.inv_view * vec4<f32>(dir_view, 0.0)).xyz);

    let r = atm.view.x;
    var color = sky_view_lookup(d, r);

    // Sun disc: radiance = E * T / solid angle, with the disc's solid angle pi * theta^2.
    let angle = max(atm.sun_irradiance.w, 1e-3);
    if (dot(d, atm.sun_dir.xyz) > cos(angle)) {
        let t_sun = transmittance_lookup(r, atm.sun_dir.y);
        let radiance = atm.sun_irradiance.xyz * t_sun / (PI * angle * angle);
        color = color + min(radiance, vec3<f32>(SUN_DISK_MAX));
    }
    return vec4<f32>(color, 1.0);
}

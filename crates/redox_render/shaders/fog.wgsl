// Height fog as a post effect. Mirrors optical_depth, transmittance, hg_phase and the fog
// colour in src/post/fog.rs; keep in sync.
// Standard depth convention: 0 at the near plane, 1 at the far plane.

struct FogUniform {
    color_density : vec4<f32>,   // rgb ambient colour, w density
    falloff       : vec4<f32>,   // x height falloff, y base height, z start distance, w max opacity
    scatter       : vec4<f32>,   // x sun scatter, y anisotropy, z near, w far
    camera_pos    : vec4<f32>,
    sun_dir       : vec4<f32>,   // direction the sun shines in (zero when unknown)
    sun_color     : vec4<f32>,   // sun colour times intensity
    inv_view_proj : mat4x4<f32>,
};

@group(0) @binding(0) var t_color  : texture_2d<f32>;
@group(0) @binding(1) var t_depth  : texture_depth_2d;
@group(0) @binding(2) var s_linear : sampler;
@group(0) @binding(3) var<uniform>   fog : FogUniform;

const PI : f32 = 3.14159265;

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

// View-space depth of a standard depth value.
fn linear_distance(depth: f32) -> f32 {
    let near = fog.scatter.z;
    let far = fog.scatter.w;
    return near * far / (far - depth * (far - near));
}

// World-space point at ndc (xy) and ndc depth z.
fn unproject(ndc_xy: vec2<f32>, ndc_z: f32) -> vec3<f32> {
    let p = fog.inv_view_proj * vec4<f32>(ndc_xy, ndc_z, 1.0);
    return p.xyz / p.w;
}

// Unit world-space direction of the view ray through a uv.
fn view_ray(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return normalize(unproject(ndc, 1.0) - unproject(ndc, 0.0));
}

// Optical depth along a ray of `distance` metres from the camera at height `y0` rising by
// `dy_per_m` per metre. The start distance is applied here, as in optical_depth.
fn optical_depth(distance: f32, y0: f32, dy_per_m: f32) -> f32 {
    let start = fog.falloff.z;
    let d = max(distance - start, 0.0);
    let y = y0 + dy_per_m * start;
    let density = fog.color_density.w * exp(-fog.falloff.x * (y - fog.falloff.y));
    let k = fog.falloff.x * dy_per_m;
    if (abs(k) < 1e-4) {
        return density * d;
    }
    return density * (1.0 - exp(-k * d)) / k;
}

fn hg_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    return (1.0 - g2) / (4.0 * PI * pow(denom, 1.5));
}

fn depth_px(uv: vec2<f32>) -> vec2<i32> {
    let dims = vec2<i32>(textureDimensions(t_depth));
    return clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let scene = textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    let depth = textureLoad(t_depth, depth_px(in.uv), 0);
    // Sky has no depth: it gets the far distance with half the optical depth.
    let sky = depth >= 1.0;
    let ray = view_ray(in.uv);
    let fwd = view_ray(vec2<f32>(0.5, 0.5));
    var dist = linear_distance(depth) / max(dot(ray, fwd), 1e-4);
    if (sky) {
        dist = fog.scatter.w;
    }
    var tau = optical_depth(dist, fog.camera_pos.y, ray.y);
    if (sky) {
        tau = tau * 0.5;
    }
    let t = exp(-tau);

    // Sun in-scatter: strongest when looking towards the sun (direction -sun_dir).
    let phase = hg_phase(dot(ray, -fog.sun_dir.xyz), fog.scatter.y);
    let inscatter = fog.scatter.x * fog.sun_color.rgb * phase * (1.0 - t);
    let fog_color = fog.color_density.rgb + inscatter;

    // Fog never covers more than max_opacity of the pixel.
    let keep_w = max(t, 1.0 - fog.falloff.w);
    return vec4<f32>(mix(fog_color, scene.rgb, keep_w), scene.a);
}

// Froxel fog composite (PST-06 step 3): replaces the analytic fog of fog.wgsl when the froxel
// volume is on. Samples the integrated grid at the pixel's view depth and blends
// scene * T + in-scatter. Mirrors fog_inject.wgsl and src/post/fog_froxel.rs; keep in sync.
// Standard depth convention: 0 at the near plane, 1 at the far plane.

struct FogUniform {
    color_density : vec4<f32>,
    falloff       : vec4<f32>,   // w max opacity
    scatter       : vec4<f32>,   // z near, w far
    camera_pos    : vec4<f32>,
    sun_dir       : vec4<f32>,
    sun_color     : vec4<f32>,
    inv_view_proj : mat4x4<f32>,
};

const FROXEL_D_F: f32 = 64.0;
const FROXEL_MAX_DISTANCE: f32 = 120.0;

@group(0) @binding(0) var t_color  : texture_2d<f32>;
@group(0) @binding(1) var t_depth  : texture_depth_2d;
@group(0) @binding(2) var t_volume : texture_3d<f32>;
@group(0) @binding(3) var s_linear : sampler;
@group(0) @binding(4) var<uniform>   fog : FogUniform;

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

fn froxel_far() -> f32 {
    return min(fog.scatter.w, FROXEL_MAX_DISTANCE);
}

// View depth of a standard depth value.
fn linear_depth(depth: f32) -> f32 {
    let near = fog.scatter.z;
    let far = fog.scatter.w;
    return near * far / (far - depth * (far - near));
}

// Continuous slice index of view depth z (inverse of slice_depth): 0 at near, FROXEL_D_F at far.
fn slice_coord(z: f32) -> f32 {
    let near = fog.scatter.z;
    return FROXEL_D_F * log(max(z, near) / near) / log(froxel_far() / near);
}

fn depth_px(uv: vec2<f32>) -> vec2<i32> {
    let dims = vec2<i32>(textureDimensions(t_depth));
    return clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let scene = textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    let depth = textureLoad(t_depth, depth_px(in.uv), 0);
    // Sky has no depth: it gets the far end of the grid, with the square root of its transmittance.
    let sky = depth >= 1.0;
    var z = froxel_far();
    if (!sky) {
        z = linear_depth(depth);
    }
    // Texel k holds the value at the end of slice k, whose texel centre is at k + 0.5.
    let w = clamp((slice_coord(z) - 0.5) / FROXEL_D_F, 0.0, 1.0);
    let lit = textureSampleLevel(t_volume, s_linear, vec3<f32>(in.uv, w), 0.0);
    var trans = lit.a;
    if (sky) {
        trans = sqrt(trans);
    }
    // Fog never covers more than max_opacity of the pixel.
    let keep = max(trans, 1.0 - fog.falloff.w);
    let scale = (1.0 - keep) / max(1.0 - trans, 1e-6);
    return vec4<f32>(scene.rgb * keep + lit.rgb * scale, scene.a);
}

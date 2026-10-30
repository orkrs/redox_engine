// Alpha-tested variant of shadow.wgsl (MAT-06 step 3): same transform, plus a fragment stage
// that discards texels whose alpha is below 0.5 (no dither: shadows do not need the noise).
struct ShadowMatrix {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> shadow: ShadowMatrix;

struct Model {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> models: array<Model>;

// First rows of the material uniform (`Material` in material_surface.wgsl).
struct MaterialHead {
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    metallic: f32,
    roughness: f32,
    flags: u32,
    sheen: f32,
};
@group(2) @binding(0) var<uniform> material: MaterialHead;
@group(2) @binding(1) var t_albedo: texture_2d<f32>;
@group(2) @binding(4) var s_material: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) alpha: f32,
};

@vertex
fn vs_main(
    @location(0) pos_q: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @builtin(instance_index) instance_idx: u32,
) -> VertexOutput {
    var out: VertexOutput;
    let mdl = models[instance_idx];
    let local_pos = decode_local_position(pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    out.clip_pos = shadow.view_proj * mdl.model * vec4<f32>(local_pos, 1.0);
    out.uv = uv;
    out.alpha = mdl.color.a;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) {
    var alpha = in.alpha;
    if ((material.flags & 1u) != 0u) {
        alpha = alpha * textureSampleLevel(t_albedo, s_material, in.uv, 0.0).a;
    }
    if (alpha < 0.5) {
        discard;
    }
}

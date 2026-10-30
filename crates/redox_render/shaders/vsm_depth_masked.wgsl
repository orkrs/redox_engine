// Alpha-tested variant of vsm_depth.wgsl (MAT-06 step 3): discards texels with alpha < 0.5.
struct Model {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

struct PageDrawInfo {
    view_proj: mat4x4<f32>,
    atlas_offset_x: f32,
    atlas_offset_y: f32,
    page_size_px: f32,
    atlas_size_px: f32,
};

@group(0) @binding(0) var<storage, read> models: array<Model>;
@group(0) @binding(1) var<uniform> page_info: PageDrawInfo;

// First rows of the material uniform (`Material` in material_surface.wgsl).
struct MaterialHead {
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    metallic: f32,
    roughness: f32,
    flags: u32,
    sheen: f32,
};
@group(1) @binding(0) var<uniform> material: MaterialHead;
@group(1) @binding(1) var t_albedo: texture_2d<f32>;
@group(1) @binding(4) var s_material: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) depth: f32,
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) alpha: f32,
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
    let world_pos = mdl.model * vec4<f32>(local_pos, 1.0);
    out.clip_pos = page_info.view_proj * world_pos;
    out.depth = out.clip_pos.z / out.clip_pos.w;
    out.uv = uv;
    out.alpha = mdl.color.a;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) f32 {
    var alpha = in.alpha;
    if ((material.flags & 1u) != 0u) {
        alpha = alpha * textureSampleLevel(t_albedo, s_material, in.uv, 0.0).a;
    }
    if (alpha < 0.5) {
        discard;
    }
    return in.depth;
}

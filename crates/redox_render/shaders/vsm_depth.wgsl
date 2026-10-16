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

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) depth: f32,
};

@vertex
fn vs_main(
    @location(0) pos_q: vec4<f32>,
    @builtin(instance_index) instance_idx: u32,
) -> VertexOutput {
    var out: VertexOutput;
    let mdl = models[instance_idx];
    let local_pos = decode_local_position(pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    let world_pos = mdl.model * vec4<f32>(local_pos, 1.0);
    out.clip_pos = page_info.view_proj * world_pos;
    out.depth = out.clip_pos.z / out.clip_pos.w;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) f32 {
    return in.depth;
}

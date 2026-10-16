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

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) pos_q: vec4<f32>,
    @builtin(instance_index) instance_idx: u32,
) -> VertexOutput {
    var out: VertexOutput;
    let mdl = models[instance_idx];
    let local_pos = decode_local_position(pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    out.clip_pos = shadow.view_proj * mdl.model * vec4<f32>(local_pos, 1.0);
    return out;
}

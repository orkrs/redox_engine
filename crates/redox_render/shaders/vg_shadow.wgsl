// Depth-only VG shadow pass — meshlet indirect draws with per-instance transforms.

struct ShadowMatrix {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> shadow: ShadowMatrix;

struct VGInstance {
    transform: mat4x4<f32>,
    meshlet_offset: u32,
    meshlet_count: u32,
    flags: u32,
    _pad: u32,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};
@group(1) @binding(0) var<storage, read> vg_instances: array<VGInstance>;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
};

@vertex
fn vs_main(in: CompressedVertIn, @builtin(instance_index) inst_idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let inst = vg_instances[inst_idx];
    let local_pos = decode_local_position(in.pos_q, inst.bounds_min.xyz, inst.bounds_extent.xyz);
    let world_pos = inst.transform * vec4<f32>(local_pos, 1.0);
    out.clip_pos = shadow.view_proj * world_pos;
    return out;
}

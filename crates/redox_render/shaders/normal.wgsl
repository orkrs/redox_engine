struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct Model {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    color: vec4<f32>,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> models: array<Model>;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
};

const FAR: f32 = 1000.0;

@vertex
fn vs_main(in: CompressedVertIn, @builtin(instance_index) instance_idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let mdl = models[instance_idx];
    let local_pos = decode_local_position(in.pos_q, mdl.bounds_min.xyz, mdl.bounds_extent.xyz);
    let world_pos = mdl.model * vec4<f32>(local_pos, 1.0);
    out.clip_pos = camera.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    let nm = mdl.normal_matrix;
    out.world_normal = normalize(nm * oct_decode(in.normal_oct));
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Positive view-space distance along the projection ray (clip.w ≈ |view_z|
    // for RH perspective). Prefer this over radial length() so SSAO samples
    // share a plane-aligned depth metric.
    let clip = camera.view_proj * vec4<f32>(in.world_pos, 1.0);
    let view_z = max(abs(clip.w), 1e-5);
    let linear_depth = clamp(view_z / FAR, 0.0, 1.0);
    return vec4<f32>(normalize(in.world_normal), linear_depth);
}

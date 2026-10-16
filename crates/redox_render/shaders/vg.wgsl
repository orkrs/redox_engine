// ── Shared structures ────────────────────────────────────────────────────

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct VGInstance {
    transform: mat4x4<f32>,
    meshlet_offset: u32,
    meshlet_count: u32,
    flags: u32,
    _pad: u32,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

// ── Bindings ─────────────────────────────────────────────────────────────

@group(0) @binding(0) var<uniform> camera: CameraUniform;

@group(1) @binding(0) var<storage, read> vg_instances: array<VGInstance>;

struct VertOut {
    @builtin(position) clip_pos:    vec4<f32>,
    @location(0)       world_pos:   vec3<f32>,
    @location(1)       world_norm:  vec3<f32>,
    @location(2)       uv:          vec2<f32>,
};

// ── Vertex shader ─────────────────────────────────────────────────────────

@vertex
fn vs_main(
    in: CompressedVertIn,
    @builtin(instance_index) inst_idx: u32,
) -> VertOut {
    let inst = vg_instances[inst_idx];
    let local_pos = decode_local_position(in.pos_q, inst.bounds_min.xyz, inst.bounds_extent.xyz);
    let world_pos  = inst.transform * vec4<f32>(local_pos, 1.0);
    let world_norm = normalize((inst.transform * vec4<f32>(oct_decode(in.normal_oct), 0.0)).xyz);

    var out: VertOut;
    out.clip_pos   = camera.view_proj * world_pos;
    out.world_pos  = world_pos.xyz;
    out.world_norm = world_norm;
    out.uv         = in.uv;
    return out;
}

// ── Fragment shader ───────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    let n_col = in.world_norm * 0.5 + vec3<f32>(0.5);
    let tint = vec3<f32>(0.2, 0.85, 0.9);
    let colour = mix(n_col, tint, 0.35);
    return vec4<f32>(colour, 1.0);
}

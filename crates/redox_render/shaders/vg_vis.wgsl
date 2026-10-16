// ── Virtual Geometry: VISIBILITY BUFFER PASS ─────────────────────────────
//
// Rasterizes VG meshlets into three attachments that the deferred resolve
// pass (vg_resolve.wgsl) shades with the full PBR lighting stack:
//
//   RT0  r32uint   bitfield  = (instance_index << 16) | triangle_id
//                  triangle_id = @builtin(primitive_index) < 124 (meshlet
//                  triangle cap), instance_index < 4096 (MAX_VG_INSTANCES).
//                  Cleared to 0xFFFFFFFF (SENTINEL) = "no VG geometry".
//   RT1  rgba16float = world-space normal.xyz + NDC depth (w)
//   RT2  rg16float   = interpolated mesh UV
//
// NOTE ON THE ID PACKING: the draw commands produced by the CPU culling
// pass carry `first_instance = instance_index`, and `@builtin(instance_index)`
// starts at `firstInstance` — so the instance index is the only per-draw
// value that reaches this shader. Every meshlet of an asset shares one
// `material_index` (see `build_vg_asset`), so the resolve pass only needs
// the instance to recover the material: `meshlets[instances[i].meshlet_offset]`.
//
// Bindings:
//   group(0) = the PBR global bind group (only `camera` is read here;
//              unused entries in the layout are fine).
//   group(1) = VG per-instance storage (`VGInstanceData`), binding 0.

enable primitive_index;

struct Camera {
    // Must mirror rust CameraUniform exactly (464 bytes).
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};

/// Mirror of `virtual_geometry::meshlet::VGInstanceData` (80 bytes).
struct VGInstance {
    transform: mat4x4<f32>,
    meshlet_offset: u32,
    meshlet_count: u32,
    flags: u32,
    _pad: u32,
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> vg_instances: array<VGInstance>;

/// RT0 value written for background pixels (also the clear value used by
/// the CPU when creating RT0).
const SENTINEL: u32 = 0xFFFFFFFFu;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    /// Global VG instance index (flat — constant across the primitive).
    @location(0) @interpolate(flat) instance_id: u32,
    /// World-space normal (interpolated).
    @location(1) world_normal: vec3<f32>,
    /// Mesh UV (interpolated).
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: CompressedVertIn, @builtin(instance_index) inst_idx: u32) -> VertexOutput {
    let inst = vg_instances[inst_idx];

    var out: VertexOutput;
    let local_pos = decode_local_position(in.pos_q, inst.bounds_min.xyz, inst.bounds_extent.xyz);
    let world_pos = inst.transform * vec4<f32>(local_pos, 1.0);
    out.clip_pos = camera.view_proj * world_pos;
    out.world_normal = normalize(mat3x3<f32>(
        inst.transform[0].xyz,
        inst.transform[1].xyz,
        inst.transform[2].xyz,
    ) * oct_decode(in.normal_oct));
    out.uv = in.uv;
    out.instance_id = inst_idx;
    return out;
}

struct FragmentOutput {
    @location(0) vis_id:     u32,
    @location(1) normal_depth: vec4<f32>,
    @location(2) uv:         vec2<f32>,
};

@fragment
fn fs_main(
    in: VertexOutput,
    @builtin(primitive_index) primitive_idx: u32,
) -> FragmentOutput {
    var out: FragmentOutput;

    // Triangle ids fit 16 bits (meshlet cap = 124 triangles); instance ids
    // fit the upper 16 bits (MAX_VG_INSTANCES = 4096).
    out.vis_id = (in.instance_id << 16u) | (primitive_idx & 0xFFFFu);

    // NDC depth from the hardware-interpolated clip position.
    out.normal_depth = vec4<f32>(in.world_normal, in.clip_pos.z);

    out.uv = in.uv;
    return out;
}

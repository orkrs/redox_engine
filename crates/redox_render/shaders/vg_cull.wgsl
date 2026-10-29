// GPU meshlet culling (two-phase).
// Phase 1: frustum + cone + Hi-Z vs previous frame.
// Phase 2: re-test Hi-Z rejects against the freshly built pyramid (same frame).
// `CullParams`, `test_sphere`, `max_scale` and `test_hiz_occlusion` come from `vg_common.wgsl`,
// which is concatenated in front of this file.

struct Meshlet {
    vertex_offset: u32,
    vertex_count: u32,
    index_offset: u32,
    index_count: u32,
    sphere_center: vec3<f32>,
    sphere_radius: f32,
    cone_axis: vec3<f32>,
    cone_cutoff: f32,
    lod_error: f32,
    lod_level: u32,
    material_index: u32,
    _pad: u32,
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

struct DrawIndexedIndirectCmd {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
};

struct RejectRecord {
    inst_idx: u32,
    meshlet_idx: u32,
};

@group(0) @binding(0) var<uniform> cull_params: CullParams;
@group(0) @binding(1) var<storage, read> instances: array<VGInstance>;
@group(0) @binding(2) var<storage, read> meshlets: array<Meshlet>;
@group(0) @binding(3) var<storage, read_write> commands: array<DrawIndexedIndirectCmd>;
@group(0) @binding(4) var<storage, read_write> cmd_count: atomic<u32>;
@group(0) @binding(5) var t_hiz: texture_2d<f32>;
@group(0) @binding(6) var s_hiz: sampler;
@group(0) @binding(7) var<storage, read_write> rejects: array<RejectRecord>;
@group(0) @binding(8) var<storage, read_write> reject_count: atomic<u32>;

fn emit_draw(inst_idx: u32, meshlet: Meshlet) {
    let cap = arrayLength(&commands);
    let slot = atomicAdd(&cmd_count, 1u);
    if (slot >= cap) {
        return;
    }
    commands[slot].index_count = meshlet.index_count;
    commands[slot].instance_count = 1u;
    commands[slot].first_index = meshlet.index_offset;
    commands[slot].base_vertex = 0;
    commands[slot].first_instance = inst_idx;
}

fn push_reject(inst_idx: u32, meshlet_idx: u32) {
    let cap = cull_params.reject_capacity;
    let slot = atomicAdd(&reject_count, 1u);
    if (slot >= cap) {
        return;
    }
    rejects[slot].inst_idx = inst_idx;
    rejects[slot].meshlet_idx = meshlet_idx;
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (cull_params.cull_mode == 1u) {
        let ri = gid.x;
        let n = atomicLoad(&reject_count);
        if (ri >= n) {
            return;
        }
        let rec = rejects[ri];
        let inst = instances[rec.inst_idx];
        let meshlet = meshlets[rec.meshlet_idx];
        let scale = max_scale(inst.transform);
        let wc = (inst.transform * vec4<f32>(meshlet.sphere_center, 1.0)).xyz;
        let wr = meshlet.sphere_radius * scale;
        if (test_hiz_occlusion(wc, wr, cull_params.view_proj)) {
            emit_draw(rec.inst_idx, meshlet);
        }
        return;
    }

    let inst_idx = gid.x;
    if (inst_idx >= arrayLength(&instances)) {
        return;
    }
    let inst = instances[inst_idx];
    if (inst.meshlet_count == 0u) {
        return;
    }

    let selected_lod = inst.flags;
    let scale = max_scale(inst.transform);
    let start = inst.meshlet_offset;
    let end = start + inst.meshlet_count;

    for (var mi = start; mi < end; mi++) {
        let meshlet = meshlets[mi];
        if (meshlet.lod_level != selected_lod) {
            continue;
        }
        let wc = (inst.transform * vec4<f32>(meshlet.sphere_center, 1.0)).xyz;
        let wr = meshlet.sphere_radius * scale;
        if (!test_sphere(wc, wr)) {
            continue;
        }
        let cone_axis = normalize(mat3x3<f32>(
            inst.transform[0].xyz,
            inst.transform[1].xyz,
            inst.transform[2].xyz,
        ) * meshlet.cone_axis);
        let to_cam = normalize(cull_params.camera_pos.xyz - wc);
        if (dot(cone_axis, to_cam) < meshlet.cone_cutoff) {
            continue;
        }
        if (!test_hiz_occlusion(wc, wr, cull_params.view_proj)) {
            push_reject(inst_idx, mi);
            continue;
        }
        emit_draw(inst_idx, meshlet);
    }
}

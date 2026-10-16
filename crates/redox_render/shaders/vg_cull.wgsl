// GPU meshlet culling (two-phase).
// Phase 1: frustum + cone + Hi-Z vs previous frame.
// Phase 2: re-test Hi-Z rejects against the freshly built pyramid (same frame).

struct CullParams {
    planes: array<vec4<f32>, 6>,
    camera_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    hiz_size: vec2<f32>,
    hiz_mip_count: u32,
    hiz_enabled: u32,
    cull_mode: u32, // 0 = phase 1, 1 = phase 2
    reject_capacity: u32,
    _pad: vec2<u32>,
};

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

fn test_sphere(center: vec3<f32>, radius: f32) -> bool {
    for (var i = 0; i < 6; i++) {
        let p = cull_params.planes[i];
        let d = dot(p.xyz, center) + p.w;
        if (d < -radius) {
            return false;
        }
    }
    return true;
}

fn max_scale(m: mat4x4<f32>) -> f32 {
    return max(length(m[0].xyz), max(length(m[1].xyz), length(m[2].xyz)));
}

fn hiz_mip_for_rect(rect_size: vec2<f32>, tex_size: vec2<f32>) -> u32 {
    let max_side = max(rect_size.x, rect_size.y);
    let level = ceil(log2(max(max_side, 1.0) / max(tex_size.x, 1.0)));
    return u32(clamp(level, 0.0, f32(cull_params.hiz_mip_count - 1u)));
}

fn test_hiz_occlusion(center: vec3<f32>, radius: f32, view_proj: mat4x4<f32>) -> bool {
    if (cull_params.hiz_enabled == 0u) {
        return true;
    }

    let clip = view_proj * vec4<f32>(center, 1.0);
    if (clip.w <= 0.0) {
        return false;
    }
    let ndc = clip.xyz / clip.w;
    if (abs(ndc.x) > 1.0 + radius || abs(ndc.y) > 1.0 + radius) {
        return false;
    }

    let uv = ndc.xy * 0.5 + 0.5;
    let px_radius = radius * cull_params.hiz_size.y * 0.5 / clip.w;
    let rect_min = uv * cull_params.hiz_size - vec2<f32>(px_radius);
    let rect_max = uv * cull_params.hiz_size + vec2<f32>(px_radius);
    let rect_size = max(rect_max - rect_min, vec2<f32>(1.0));
    let mip = hiz_mip_for_rect(rect_size, cull_params.hiz_size);

    var max_hiz = 0.0;
    for (var sy = 0u; sy < 2u; sy++) {
        for (var sx = 0u; sx < 2u; sx++) {
            let su = (f32(sx) + 0.5) / 2.0;
            let sv = (f32(sy) + 0.5) / 2.0;
            let sample_uv = mix(rect_min, rect_max, vec2<f32>(su, sv)) / cull_params.hiz_size;
            max_hiz = max(max_hiz, textureSampleLevel(t_hiz, s_hiz, sample_uv, f32(mip)).x);
        }
    }

    let to_cam = normalize(cull_params.camera_pos.xyz - center);
    let nearest = center + to_cam * radius;
    let nearest_clip = view_proj * vec4<f32>(nearest, 1.0);
    let nearest_ndc_z = nearest_clip.z / nearest_clip.w;
    return nearest_ndc_z <= max_hiz + 0.002;
}

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

// Level-by-level cut of cluster DAGs on the GPU (VG-02).
//
// Mirror of `cut` in `virtual_geometry/dag.rs` (the CPU reference):
//   a cluster is drawn when  own_px <= threshold  and  parent_px > threshold;
//   otherwise, when own_px > threshold, its children are queued for the next pass.
// The local eye of every instance is computed on the CPU (`dag_gpu.rs`), so the projection runs in
// the same local space as the reference.
//
// Hi-Z: a selected cluster that fails the occlusion test is recorded in `rejects`. After the
// pyramid of the current frame is built, `retest_rejects` tests those records again and draws the
// survivors (the same-frame phase 2 of the flat path).
//
// Entry points: `main` (one traversal pass), `prepare_args` (one thread; indirect dispatch sizes of
// both queues), `retest_rejects` (phase 2).
//
// Layout contract (keep in sync with `dag_gpu.rs`): `DagClusterGpu` is 96 bytes and is mirrored by
// the scalar fields of `DagCluster` (vec3 would be 16-aligned and shift the offsets). `DagInstanceGpu`
// is 112 bytes, `DagPassParams` 32 bytes, `PrepareParams` 16 bytes.
// `CullParams`, `test_sphere`, `max_scale` and `test_hiz_occlusion` come from `vg_common.wgsl`.

// Matches `DagClusterGpu` (96 bytes). Field order is the Rust field order.
struct DagCluster {
    index_offset: u32,
    index_count: u32,
    cx: f32,
    cy: f32,
    cz: f32,
    radius: f32,
    lcx: f32,
    lcy: f32,
    lcz: f32,
    lradius: f32,
    error: f32,
    pcx: f32,
    pcy: f32,
    pcz: f32,
    pradius: f32,
    parent_error: f32,
    level: u32,
    group: u32,
    first_child: u32,
    child_count: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
    _p3: u32,
};

// Matches `DagInstanceGpu` (112 bytes).
struct DagInstanceGpu {
    transform: mat4x4<f32>,
    local_eye: vec4<f32>,
    bound: vec4<f32>,
    root_begin: u32,
    root_count: u32,
    vertex_base: u32,
    _pad: u32,
};

// Matches `DagPassParams` (32 bytes).
struct DagPassParams {
    pixels_per_unit: f32,
    threshold_px: f32,
    queue_capacity: u32,
    cull_frustum: u32,
    src_slot: u32,
    dst_slot: u32,
    _pad: vec2<u32>,
};

// Matches `PrepareParams` (16 bytes).
struct PrepareParams {
    queue_capacity: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct DrawIndexedIndirectCmd {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
};

// Counter slots: [0] queue A count, [1] queue B count, [2] draw commands,
// [3] overflow events, [4] Hi-Z rejects.
const COUNTER_CMD: u32 = 2u;
const COUNTER_OVERFLOW: u32 = 3u;
const COUNTER_REJECT: u32 = 4u;
// Stand-in for an infinite parent error of a root (kept finite on purpose).
const ROOT_PARENT_PX: f32 = 3.0e38;

@group(0) @binding(0) var<uniform> cull_params: CullParams;
@group(0) @binding(1) var<uniform> dag_params: DagPassParams;
@group(0) @binding(2) var<storage, read> clusters: array<DagCluster>;
@group(0) @binding(3) var<storage, read> child_ids: array<u32>;
@group(0) @binding(4) var<storage, read> instances: array<DagInstanceGpu>;
@group(0) @binding(5) var<storage, read> src_queue: array<vec2<u32>>;
@group(0) @binding(6) var<storage, read_write> dst_queue: array<vec2<u32>>;
@group(0) @binding(7) var<storage, read_write> counters: array<atomic<u32>, 5>;
@group(0) @binding(8) var<storage, read_write> commands: array<DrawIndexedIndirectCmd>;
@group(0) @binding(9) var t_hiz: texture_2d<f32>;
@group(0) @binding(10) var s_hiz: sampler;
// Records `(instance, cluster)` of clusters rejected by Hi-Z, for the phase-2 retest.
@group(0) @binding(11) var<storage, read_write> rejects: array<vec2<u32>>;

// Prepare-pass bindings (separate bind group layout; the counters are shared).
@group(0) @binding(12) var<storage, read_write> dispatch_args: array<u32>;
@group(0) @binding(13) var<uniform> prepare_params: PrepareParams;

fn cluster_centre(c: DagCluster) -> vec3<f32> {
    return vec3<f32>(c.cx, c.cy, c.cz);
}

fn cluster_lod_centre(c: DagCluster) -> vec3<f32> {
    return vec3<f32>(c.lcx, c.lcy, c.lcz);
}

fn cluster_parent_centre(c: DagCluster) -> vec3<f32> {
    return vec3<f32>(c.pcx, c.pcy, c.pcz);
}

// Projected size in pixels of an object-space error, as `projected_error_px` on the CPU.
fn projected_px(error: f32, centre: vec3<f32>, radius: f32, eye: vec3<f32>) -> f32 {
    let d = max(distance(centre, eye) - radius, 0.001);
    return error * dag_params.pixels_per_unit / d;
}

fn emit_cluster(inst_idx: u32, base_vertex: i32, c: DagCluster) {
    let slot = atomicAdd(&counters[COUNTER_CMD], 1u);
    if (slot >= arrayLength(&commands)) {
        return;
    }
    commands[slot].index_count = c.index_count;
    commands[slot].instance_count = 1u;
    commands[slot].first_index = c.index_offset;
    commands[slot].base_vertex = base_vertex;
    commands[slot].first_instance = inst_idx;
}

fn push_reject(inst_idx: u32, cluster_id: u32) {
    let slot = atomicAdd(&counters[COUNTER_REJECT], 1u);
    if (slot >= dag_params.queue_capacity) {
        return;
    }
    rejects[slot] = vec2<u32>(inst_idx, cluster_id);
}

// Draws a selected cluster when it passes the frustum test; records a reject when Hi-Z occludes it.
fn select_cluster(inst_idx: u32, cluster_id: u32, inst: DagInstanceGpu, c: DagCluster) {
    let wc = (inst.transform * vec4<f32>(cluster_centre(c), 1.0)).xyz;
    let wr = c.radius * max_scale(inst.transform);
    if (dag_params.cull_frustum != 0u && !test_sphere(wc, wr)) {
        return;
    }
    if (!test_hiz_occlusion(wc, wr, cull_params.view_proj)) {
        push_reject(inst_idx, cluster_id);
        return;
    }
    emit_cluster(inst_idx, i32(inst.vertex_base), c);
}

// Queues the children of `c`. When the destination queue is full, `c` itself is selected instead
// (coarser, never dropped).
fn push_children(inst_idx: u32, cluster_id: u32, inst: DagInstanceGpu, c: DagCluster) {
    let n = c.child_count;
    let slot = atomicAdd(&counters[dag_params.dst_slot], n);
    if (slot + n > dag_params.queue_capacity) {
        atomicAdd(&counters[COUNTER_OVERFLOW], 1u);
        select_cluster(inst_idx, cluster_id, inst, c);
        return;
    }
    for (var i = 0u; i < n; i++) {
        dst_queue[slot + i] = vec2<u32>(inst_idx, child_ids[c.first_child + i]);
    }
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let queued = min(atomicLoad(&counters[dag_params.src_slot]), dag_params.queue_capacity);
    if (gid.x >= queued) {
        return;
    }
    let item = src_queue[gid.x];
    let inst = instances[item.x];
    let c = clusters[item.y];
    let eye = inst.local_eye.xyz;

    let own_px = projected_px(c.error, cluster_lod_centre(c), c.lradius, eye);
    var parent_px = ROOT_PARENT_PX;
    if (c.parent_error < ROOT_PARENT_PX) {
        parent_px = projected_px(c.parent_error, cluster_parent_centre(c), c.pradius, eye);
    }

    // The parent is drawn instead of this cluster (or this cluster is not reached at all).
    if (parent_px <= dag_params.threshold_px) {
        return;
    }
    // Precise enough (or a leaf): this cluster is the cut.
    if (own_px <= dag_params.threshold_px || c.child_count == 0u) {
        select_cluster(item.x, item.y, inst, c);
        return;
    }
    push_children(item.x, item.y, inst, c);
}

@compute @workgroup_size(1, 1, 1)
fn prepare_args() {
    for (var s = 0u; s < 2u; s++) {
        let n = min(atomicLoad(&counters[s]), prepare_params.queue_capacity);
        dispatch_args[s * 3u] = (n + 63u) / 64u;
        dispatch_args[s * 3u + 1u] = 1u;
        dispatch_args[s * 3u + 2u] = 1u;
    }
}

// Phase 2: re-tests the Hi-Z rejects of this frame against the freshly built pyramid.
@compute @workgroup_size(64, 1, 1)
fn retest_rejects(@builtin(global_invocation_id) gid: vec3<u32>) {
    let n = min(atomicLoad(&counters[COUNTER_REJECT]), dag_params.queue_capacity);
    if (gid.x >= n) {
        return;
    }
    let r = rejects[gid.x];
    let inst = instances[r.x];
    let c = clusters[r.y];
    let wc = (inst.transform * vec4<f32>(cluster_centre(c), 1.0)).xyz;
    let wr = c.radius * max_scale(inst.transform);
    if (test_hiz_occlusion(wc, wr, cull_params.view_proj)) {
        emit_cluster(r.x, i32(inst.vertex_base), c);
    }
}

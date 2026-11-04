// Foliage layer culling: frustum test, distance LOD with hysteresis, one indirect draw per LOD.
// `main` writes the visible instance indices and the per-LOD counts; `finalize` turns the counts
// into DrawIndexedIndirect records. The CPU reference is `foliage::layer::cull_reference`.

const MAX_LODS: u32 = 4u;
const NO_LOD: u32 = 0xffffffffu;
const LOD_HYSTERESIS: f32 = 0.05;
const INDIRECT_WORDS: u32 = 5u;

struct FoliageInstance {
    pos: vec3<f32>,
    yaw: f32,
    scale: f32,
    seed: u32,
    _pad: vec2<u32>,
};

struct CullParams {
    planes: array<vec4<f32>, 6>,
    camera_pos: vec4<f32>,
    lod_max_distance: vec4<f32>,
    lod_index_count: vec4<u32>,
    lod_count: u32,
    instance_count: u32,
    capacity: u32,
    bound_radius: f32,
};

@group(0) @binding(0) var<uniform> params: CullParams;
@group(0) @binding(1) var<storage, read> instances: array<FoliageInstance>;
@group(0) @binding(2) var<storage, read_write> prev_lod: array<u32>;
@group(0) @binding(3) var<storage, read_write> lod_counts: array<atomic<u32>, MAX_LODS>;
@group(0) @binding(4) var<storage, read_write> visible: array<u32>;
@group(0) @binding(5) var<storage, read_write> indirect: array<u32>;

fn lod_far(lod: u32) -> f32 {
    if (lod == 0u) { return params.lod_max_distance.x; }
    if (lod == 1u) { return params.lod_max_distance.y; }
    if (lod == 2u) { return params.lod_max_distance.z; }
    return params.lod_max_distance.w;
}

fn lod_index_count(lod: u32) -> u32 {
    if (lod == 0u) { return params.lod_index_count.x; }
    if (lod == 1u) { return params.lod_index_count.y; }
    if (lod == 2u) { return params.lod_index_count.z; }
    return params.lod_index_count.w;
}

// Same rule as `choose_lod` on the CPU: keep the previous LOD inside its band, else the first LOD
// whose far distance covers `distance`, else NO_LOD.
fn pick_lod(distance: f32, previous: u32) -> u32 {
    let n = params.lod_count;
    if (previous < n) {
        let far_ok = distance <= lod_far(previous) * (1.0 + LOD_HYSTERESIS);
        var near_ok = true;
        if (previous > 0u) {
            near_ok = distance >= lod_far(previous - 1u) * (1.0 - LOD_HYSTERESIS);
        }
        if (far_ok && near_ok) {
            return previous;
        }
    }
    for (var l = 0u; l < n; l++) {
        if (distance <= lod_far(l)) {
            return l;
        }
    }
    return NO_LOD;
}

fn sphere_in_frustum(center: vec3<f32>, radius: f32) -> bool {
    for (var i = 0u; i < 6u; i++) {
        let p = params.planes[i];
        if (dot(p.xyz, center) + p.w < -radius) {
            return false;
        }
    }
    return true;
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.instance_count) {
        return;
    }
    let inst = instances[i];
    if (!sphere_in_frustum(inst.pos, inst.scale * params.bound_radius)) {
        return;
    }
    let offset = inst.pos - params.camera_pos.xyz;
    let lod = pick_lod(sqrt(dot(offset, offset)), prev_lod[i]);
    prev_lod[i] = lod;
    if (lod == NO_LOD) {
        return;
    }
    let slot = atomicAdd(&lod_counts[lod], 1u);
    if (slot < params.capacity) {
        visible[lod * params.capacity + slot] = i;
    }
}

@compute @workgroup_size(1, 1, 1)
fn finalize() {
    for (var l = 0u; l < MAX_LODS; l++) {
        let base = l * INDIRECT_WORDS;
        var index_count = 0u;
        var instance_count = 0u;
        if (l < params.lod_count) {
            index_count = lod_index_count(l);
            instance_count = min(atomicLoad(&lod_counts[l]), params.capacity);
        }
        indirect[base] = index_count;
        indirect[base + 1u] = instance_count;
        indirect[base + 2u] = 0u;
        indirect[base + 3u] = 0u;
        indirect[base + 4u] = 0u;
    }
}

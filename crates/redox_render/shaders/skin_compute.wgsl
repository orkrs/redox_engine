// GPU linear-blend skinning. Reproduces `SkinnedGeometry::deform_into` /
// `deform_tangents_into` followed by the 16-byte `CompressedVertex` quantisation of
// `mesh/compress.rs`. Vertices without tangents keep the fixed tangent (1, 0, 0).

struct SkinVertex {
    position: vec3<f32>,
    uv_packed: u32, // two f16, already packed on the CPU
    normal: vec3<f32>,
    _pad: u32,
    joints: vec4<u32>,
    weights: vec4<f32>,
    tangent: vec3<f32>,
    has_tangent: u32,
}

struct Params {
    bounds_min: vec4<f32>,
    bounds_extent: vec4<f32>,
    vertex_count: u32,
    matrix_count: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> bind_vertices: array<SkinVertex>;
@group(0) @binding(2) var<storage, read> skin: array<mat4x4<f32>>;
@group(0) @binding(3) var<storage, read_write> out_vertices: array<vec4<u32>>;

// Tangent (1, 0, 0) octahedral-encoded as snorm8 pair: x = 127 (low byte), y = 0 (high byte).
const FIXED_TANGENT_BITS: u32 = 0x007Fu;

fn round_half_away(x: f32) -> f32 {
    return sign(x) * floor(abs(x) + 0.5);
}

fn snorm16(x: f32) -> u32 {
    return u32(i32(round_half_away(clamp(x, -1.0, 1.0) * 32767.0))) & 0xFFFFu;
}

fn snorm8(x: f32) -> u32 {
    return u32(i32(round_half_away(clamp(x, -1.0, 1.0) * 127.0))) & 0xFFu;
}

fn oct_encode(n: vec3<f32>) -> vec2<f32> {
    let inv = max(abs(n.x) + abs(n.y) + abs(n.z), 1e-6);
    var v = n / inv;
    if (v.y < 0.0) {
        let ox = v.x;
        v.x = (1.0 - abs(v.z)) * select(-1.0, 1.0, ox >= 0.0);
        v.z = (1.0 - abs(ox)) * select(-1.0, 1.0, v.z >= 0.0);
    }
    return vec2<f32>(v.x, v.z);
}

fn blend_matrix(joints: vec4<u32>, weights: vec4<f32>) -> mat4x4<f32> {
    var sum = mat4x4<f32>(
        vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
    var total = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let w = weights[i];
        let j = joints[i];
        if (w > 0.0 && j < params.matrix_count) {
            sum += skin[j] * w;
            total += w;
        }
    }
    if (total <= 1.1920929e-7) {
        return mat4x4<f32>(
            vec4<f32>(1.0, 0.0, 0.0, 0.0),
            vec4<f32>(0.0, 1.0, 0.0, 0.0),
            vec4<f32>(0.0, 0.0, 1.0, 0.0),
            vec4<f32>(0.0, 0.0, 0.0, 1.0));
    }
    return sum * (1.0 / total);
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.vertex_count) {
        return;
    }
    let v = bind_vertices[i];
    let m = blend_matrix(v.joints, v.weights);
    let p = (m * vec4<f32>(v.position, 1.0)).xyz;
    var n = (m * vec4<f32>(v.normal, 0.0)).xyz;
    let len = length(n);
    if (len > 0.0) {
        n = n / len;
    } else {
        n = vec3<f32>(0.0);
    }

    let t = (p - params.bounds_min.xyz) / params.bounds_extent.xyz;
    let q = t * 2.0 - vec3<f32>(1.0);
    let handed = select(-1.0, 1.0, n.z >= 0.0);
    let o = oct_encode(n);

    var tangent_word = FIXED_TANGENT_BITS;
    if (v.has_tangent != 0u) {
        var t = (m * vec4<f32>(v.tangent, 0.0)).xyz;
        let tlen = length(t);
        if (tlen > 0.0) {
            t = t / tlen;
        } else {
            t = vec3<f32>(0.0);
        }
        let ot = oct_encode(t);
        tangent_word = snorm8(ot.x) | (snorm8(ot.y) << 8u);
    }

    let w0 = snorm16(q.x) | (snorm16(q.y) << 16u);
    let w1 = snorm16(q.z) | (snorm16(handed) << 16u);
    // Bytes 8..12 of CompressedVertex: normal oct x, y, tangent oct x, y (little endian).
    let w2 = snorm8(o.x) | (snorm8(o.y) << 8u) | (tangent_word << 16u);
    out_vertices[i] = vec4<u32>(w0, w1, w2, v.uv_packed);
}

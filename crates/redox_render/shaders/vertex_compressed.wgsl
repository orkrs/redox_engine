// Shared compressed-vertex decode (included before pass shaders).

struct CompressedVertIn {
    @location(0) pos_q: vec4<f32>,
    @location(1) normal_oct: vec2<f32>,
    @location(2) tangent_oct: vec2<f32>,
    @location(3) uv: vec2<f32>,
};

fn oct_decode(o: vec2<f32>) -> vec3<f32> {
    // VertexFormat::Snorm8x2 already expands to [-1, 1] in the VS input.
    var n = vec3<f32>(o.x, 1.0 - abs(o.x) - abs(o.y), o.y);
    let t = max(-n.y, 0.0);
    n.x += select(-t, t, o.x >= 0.0);
    n.z += select(-t, t, o.y >= 0.0);
    return normalize(n);
}

fn decode_local_position(pos_q: vec4<f32>, bounds_min: vec3<f32>, bounds_extent: vec3<f32>) -> vec3<f32> {
    let t = pos_q.xyz * 0.5 + 0.5;
    return bounds_min + t * bounds_extent;
}

fn decode_tangent_sign(pos_q: vec4<f32>) -> f32 {
    return select(-1.0, 1.0, pos_q.w >= 0.0);
}

fn decode_compressed_attributes(
    in: CompressedVertIn,
    bounds_min: vec3<f32>,
    bounds_extent: vec3<f32>,
) -> vec4<f32> {
    let local_pos = decode_local_position(in.pos_q, bounds_min, bounds_extent);
    let normal = oct_decode(in.normal_oct);
    let tangent = oct_decode(in.tangent_oct);
    let sign = decode_tangent_sign(in.pos_q);
    // Pack sign into tangent.w for downstream bitangent reconstruction.
    return vec4<f32>(local_pos, sign);
}

const PI: f32 = 3.14159265359;

@group(0) @binding(0) var output_lut: texture_storage_2d<rgba16float, write>;

fn radical_inverse_vdc(bits_in: u32) -> f32 {
    var bits = (bits_in << 16u) | (bits_in >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}
fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse_vdc(i));
}
fn importance_sample_ggx(xi: vec2<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let a  = roughness * roughness;
    let phi       = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    let h = vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(n.y) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let right = normalize(cross(up, n));
    up = cross(n, right);
    return normalize(right * h.x + up * h.y + n * h.z);
}
fn geometry_schlick_ggx_ibl(n_dot_v: f32, roughness: f32) -> f32 {
    let k = (roughness * roughness) / 2.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}
fn geometry_smith_ibl(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx_ibl(n_dot_v, roughness)
         * geometry_schlick_ggx_ibl(n_dot_l, roughness);
}

@compute @workgroup_size(8, 8, 1)
fn brdf_lut(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_lut);
    if (id.x >= size.x || id.y >= size.y) { return; }

    let n_dot_v   = max(f32(id.x) / f32(size.x), 0.001);
    let roughness = max(f32(id.y) / f32(size.y), 0.001);

    var v: vec3<f32>;
    v.x = sqrt(1.0 - n_dot_v * n_dot_v);
    v.y = 0.0;
    v.z = n_dot_v;

    var a = 0.0;
    var b = 0.0;
    let n = vec3<f32>(0.0, 0.0, 1.0);
    let sample_count = 1024u;

    for (var i = 0u; i < sample_count; i++) {
        let xi    = hammersley(i, sample_count);
        let h     = importance_sample_ggx(xi, n, roughness);
        let l     = normalize(2.0 * dot(v, h) * h - v);
        let n_dot_l = max(l.z, 0.0);
        let n_dot_h = max(h.z, 0.0);
        let v_dot_h = max(dot(v, h), 0.0);
        if (n_dot_l > 0.0) {
            let g     = geometry_smith_ibl(n_dot_v, n_dot_l, roughness);
            // Guard against division by zero
            let g_vis = (g * v_dot_h) / max(n_dot_h * n_dot_v, 0.0001);
            let fc    = pow(1.0 - v_dot_h, 5.0);
            a += (1.0 - fc) * g_vis;
            b += fc * g_vis;
        }
    }
    textureStore(output_lut, id.xy,
        vec4<f32>(a / f32(sample_count), b / f32(sample_count), 0.0, 1.0));
}

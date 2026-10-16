const PI: f32 = 3.14159265359;

@group(0) @binding(0) var environment_map:     texture_cube<f32>;
@group(0) @binding(1) var samp:                sampler;
@group(1) @binding(0) var output_prefiltered:  texture_storage_2d_array<rgba16float, write>;

struct PrefilterUniforms { roughness: f32 };
@group(2) @binding(0) var<uniform> prefilter_u: PrefilterUniforms;

fn get_cube_dir(id: vec3<u32>, size: vec2<u32>) -> vec3<f32> {
    let uv        = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    let tex_coords = uv * 2.0 - 1.0;
    var dir: vec3<f32>;
    let face = id.z;
    if      (face == 0u) { dir = vec3<f32>( 1.0, -tex_coords.y, -tex_coords.x); }
    else if (face == 1u) { dir = vec3<f32>(-1.0, -tex_coords.y,  tex_coords.x); }
    else if (face == 2u) { dir = vec3<f32>( tex_coords.x,  1.0,  tex_coords.y); }
    else if (face == 3u) { dir = vec3<f32>( tex_coords.x, -1.0, -tex_coords.y); }
    else if (face == 4u) { dir = vec3<f32>( tex_coords.x, -tex_coords.y,  1.0); }
    else                 { dir = vec3<f32>(-tex_coords.x, -tex_coords.y, -1.0); }
    return normalize(dir);
}
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

@compute @workgroup_size(8, 8, 1)
fn prefilter_convolution(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_prefiltered).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }

    let n = get_cube_dir(id, size);
    let v = n;

    let sample_count = 1024u;
    var total_weight    = 0.0;
    var prefiltered_color = vec3<f32>(0.0);

    for (var i = 0u; i < sample_count; i++) {
        let xi    = hammersley(i, sample_count);
        let h     = importance_sample_ggx(xi, n, prefilter_u.roughness);
        let l     = normalize(2.0 * dot(v, h) * h - v);
        let n_dot_l = max(dot(n, l), 0.0);
        if (n_dot_l > 0.0) {
            prefiltered_color += textureSampleLevel(environment_map, samp, l, 0.0).rgb * n_dot_l;
            total_weight      += n_dot_l;
        }
    }
    prefiltered_color /= max(total_weight, 0.0001);
    textureStore(output_prefiltered, id.xy, i32(id.z), vec4<f32>(prefiltered_color, 1.0));
}

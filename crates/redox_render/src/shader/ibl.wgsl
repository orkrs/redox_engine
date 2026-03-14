//! IBL Pre-processing shaders.

const PI: f32 = 3.14159265359;

// --- Equirectangular to Cubemap ---

@group(0) @binding(0) var input_equirect: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var output_cubemap: texture_storage_2d_array<rgba16float, write>;

fn get_cube_dir(id: vec3<u32>, size: vec2<u32>) -> vec3<f32> {
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    let tex_coords = uv * 2.0 - 1.0;
    
    var dir: vec3<f32>;
    let face = id.z;
    
    // wgpu cube face order: +X, -X, +Y, -Y, +Z, -Z
    if (face == 0u) { dir = vec3<f32>(1.0, -tex_coords.y, -tex_coords.x); } 
    else if (face == 1u) { dir = vec3<f32>(-1.0, -tex_coords.y, tex_coords.x); }
    else if (face == 2u) { dir = vec3<f32>(tex_coords.x, 1.0, tex_coords.y); }
    else if (face == 3u) { dir = vec3<f32>(tex_coords.x, -1.0, -tex_coords.y); }
    else if (face == 4u) { dir = vec3<f32>(tex_coords.x, -tex_coords.y, 1.0); }
    else { dir = vec3<f32>(-tex_coords.x, -tex_coords.y, -1.0); }
    
    return normalize(dir);
}

fn sample_equirect(v: vec3<f32>) -> vec2<f32> {
    let phi = atan2(v.z, v.x);
    let theta = asin(v.y);
    return vec2<f32>(phi / (2.0 * PI) + 0.5, theta / PI + 0.5);
}

@compute @workgroup_size(8, 8, 1)
fn equirect_to_cubemap(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_cubemap).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }
    
    let dir = get_cube_dir(id, size);
    let uv = sample_equirect(dir);
    let color = textureSampleLevel(input_equirect, samp, uv, 0.0);
    
    textureStore(output_cubemap, id.xy, i32(id.z), color);
}

// --- Irradiance Map Convolution (Diffuse IBL) ---

@group(0) @binding(0) var environment_map: texture_cube<f32>;
@group(1) @binding(0) var output_irradiance: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn irradiance_convolution(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_irradiance).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }

    let normal = get_cube_dir(id, size);
    var irradiance = vec3<f32>(0.0);

    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999) {
        up = vec3<f32>(1.0, 0.0, 0.0);
    }
    let right = normalize(cross(up, normal));
    up = cross(normal, right);

    let sample_delta = 0.025;
    var nr_samples = 0.0;
    
    for (var phi = 0.0; phi < 2.0 * PI; phi += sample_delta) {
        for (var theta = 0.0; theta < 0.5 * PI; theta += sample_delta) {
            let tangent_sample = vec3<f32>(sin(theta) * cos(phi), sin(theta) * sin(phi), cos(theta));
            let world_sample = tangent_sample.x * right + tangent_sample.y * up + tangent_sample.z * normal;

            irradiance += textureSampleLevel(environment_map, samp, world_sample, 0.0).rgb * cos(theta) * sin(theta);
            nr_samples += 1.0;
        }
    }
    
    irradiance = PI * irradiance * (1.0 / nr_samples);
    textureStore(output_irradiance, id.xy, i32(id.z), vec4<f32>(irradiance, 1.0));
}

// --- BRDF LUT Generation ---

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
    let a = roughness * roughness;
    let phi = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    
    let h = vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);
    
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(n.y) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let right = normalize(cross(up, n));
    up = cross(n, right);
    
    return normalize(right * h.x + up * h.y + n * h.z);
}

fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let k = (roughness * roughness) / 2.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

@compute @workgroup_size(8, 8, 1)
fn brdf_lut(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_lut);
    if (id.x >= size.x || id.y >= size.y) { return; }
    
    let n_dot_v = f32(id.x) / f32(size.x);
    let roughness = f32(id.y) / f32(size.y);
    
    var v: vec3<f32>;
    v.x = sqrt(1.0 - n_dot_v * n_dot_v);
    v.y = 0.0;
    v.z = n_dot_v;
    
    var a = 0.0;
    var b = 0.0;
    
    let n = vec3<f32>(0.0, 0.0, 1.0);
    let sample_count = 1024u;
    
    for (var i = 0u; i < sample_count; i++) {
        let xi = hammersley(i, sample_count);
        let h = importance_sample_ggx(xi, n, roughness);
        let l = normalize(2.0 * dot(v, h) * h - v);
        
        let n_dot_l = max(l.z, 0.0);
        let n_dot_h = max(h.z, 0.0);
        let v_dot_h = max(dot(v, h), 0.0);
        
        if (n_dot_l > 0.0) {
            let g = geometry_smith(n_dot_v, n_dot_l, roughness);
            let g_vis = (g * v_dot_h) / (n_dot_h * n_dot_v);
            let fc = pow(1.0 - v_dot_h, 5.0);
            
            a += (1.0 - fc) * g_vis;
            b += fc * g_vis;
        }
    }
    
    textureStore(output_lut, id.xy, vec4<f32>(a / f32(sample_count), b / f32(sample_count), 0.0, 1.0));
}

// --- Specular Pre-filter Map (Specular IBL) ---

@group(1) @binding(0) var output_prefiltered: texture_storage_2d_array<rgba16float, write>;

struct PrefilterUniforms {
    roughness: f32,
};
@group(2) @binding(0) var<uniform> prefilter_u: PrefilterUniforms;

@compute @workgroup_size(8, 8, 1)
fn prefilter_convolution(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_prefiltered).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }
    
    let n = get_cube_dir(id, size);
    let r = n;
    let v = r;
    
    let sample_count = 1024u;
    var total_weight = 0.0;
    var prefiltered_color = vec3<f32>(0.0);
    
    for (var i = 0u; i < sample_count; i++) {
        let xi = hammersley(i, sample_count);
        let h = importance_sample_ggx(xi, n, prefilter_u.roughness);
        let l = normalize(2.0 * dot(v, h) * h - v);
        
        let n_dot_l = max(dot(n, l), 0.0);
        if (n_dot_l > 0.0) {
            prefiltered_color += textureSampleLevel(environment_map, samp, l, 0.0).rgb * n_dot_l;
            total_weight += n_dot_l;
        }
    }
    
    prefiltered_color = prefiltered_color / total_weight;
    textureStore(output_prefiltered, id.xy, i32(id.z), vec4<f32>(prefiltered_color, 1.0));
}

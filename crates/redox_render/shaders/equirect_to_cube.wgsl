const PI: f32 = 3.14159265359;

@group(0) @binding(0) var input_equirect:  texture_2d<f32>;
@group(0) @binding(1) var samp:            sampler;
@group(0) @binding(2) var output_cubemap:  texture_storage_2d_array<rgba16float, write>;

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

fn sample_equirect(v: vec3<f32>) -> vec2<f32> {
    let phi   = atan2(v.z, v.x);
    let theta = asin(v.y);
    return vec2<f32>(phi / (2.0 * PI) + 0.5, theta / PI + 0.5);
}

@compute @workgroup_size(8, 8, 1)
fn equirect_to_cubemap(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_cubemap).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }
    let color = textureSampleLevel(input_equirect, samp, sample_equirect(get_cube_dir(id, size)), 0.0);
    textureStore(output_cubemap, id.xy, i32(id.z), color);
}

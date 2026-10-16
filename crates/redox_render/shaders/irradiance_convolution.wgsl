const PI: f32 = 3.14159265359;

@group(0) @binding(0) var environment_map:    texture_cube<f32>;
@group(0) @binding(1) var samp:               sampler;
@group(1) @binding(0) var output_irradiance:  texture_storage_2d_array<rgba16float, write>;

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

@compute @workgroup_size(8, 8, 1)
fn irradiance_convolution(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output_irradiance).xy;
    if (id.x >= size.x || id.y >= size.y) { return; }

    let normal = get_cube_dir(id, size);
    var irradiance = vec3<f32>(0.0);

    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(normal.y) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let right = normalize(cross(up, normal));
    up = cross(normal, right);

    let sample_delta = 0.025;
    var nr_samples = 0.0;
    for (var phi = 0.0; phi < 2.0 * PI; phi += sample_delta) {
        for (var theta = 0.0; theta < 0.5 * PI; theta += sample_delta) {
            let ts  = vec3<f32>(sin(theta) * cos(phi), sin(theta) * sin(phi), cos(theta));
            let ws  = ts.x * right + ts.y * up + ts.z * normal;
            irradiance += textureSampleLevel(environment_map, samp, ws, 0.0).rgb
                          * cos(theta) * sin(theta);
            nr_samples += 1.0;
        }
    }
    irradiance = PI * irradiance / nr_samples;
    textureStore(output_irradiance, id.xy, i32(id.z), vec4<f32>(irradiance, 1.0));
}

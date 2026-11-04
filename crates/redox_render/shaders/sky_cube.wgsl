// Fills a cube map with the sky-view LUT, for IBL (irradiance and prefilter convolutions).
// Rust prepends `sky_common.wgsl` and `sky_lookup.wgsl`. Dispatch: (size/8, size/8, 6).
//
// Face directions follow the WebGPU cube convention (layer order +X, -X, +Y, -Y, +Z, -Z,
// row 0 at the top of each face), so sampling by direction in the convolutions reads back
// the same texel that is written here.

@group(0) @binding(0) var<uniform> atm: Atmosphere;
// sky_lookup.wgsl references these; only t_sky_view is sampled by this entry point.
@group(0) @binding(1) var t_transmittance: texture_2d<f32>;
@group(0) @binding(2) var s_lut: sampler;
@group(0) @binding(3) var t_multiscatter: texture_2d<f32>;
@group(0) @binding(4) var t_sky_view: texture_2d<f32>;
@group(0) @binding(5) var s_sky: sampler;
@group(0) @binding(6) var t_cube: texture_storage_2d_array<rgba16float, write>;

// Unit direction of the texel centre (s, t in -1..1, t grows downwards) on face `face`.
fn cube_direction(face: u32, s: f32, t: f32) -> vec3<f32> {
    switch face {
        case 0u: { return normalize(vec3<f32>(1.0, -t, -s)); }
        case 1u: { return normalize(vec3<f32>(-1.0, -t, s)); }
        case 2u: { return normalize(vec3<f32>(s, 1.0, t)); }
        case 3u: { return normalize(vec3<f32>(s, -1.0, -t)); }
        case 4u: { return normalize(vec3<f32>(s, -t, 1.0)); }
        default: { return normalize(vec3<f32>(-s, -t, -1.0)); }
    }
}

@compute @workgroup_size(8, 8, 1)
fn sky_cube(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(t_cube);
    if (id.x >= size.x || id.y >= size.y || id.z >= 6u) {
        return;
    }
    let s = 2.0 * (f32(id.x) + 0.5) / f32(size.x) - 1.0;
    let t = 2.0 * (f32(id.y) + 0.5) / f32(size.y) - 1.0;
    let d = cube_direction(id.z, s, t);
    let color = sky_view_lookup(d, atm.view.x);
    textureStore(t_cube, vec2<i32>(id.xy), i32(id.z), vec4<f32>(color, 1.0));
}

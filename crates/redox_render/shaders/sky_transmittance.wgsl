// Transmittance LUT (Hillaire 2020, section 5.1). Rust prepends `sky_common.wgsl`.
// Output texel (x, y) holds the transmittance from the ground-relative radius r and cosine mu
// given by `transmittance_uv_to_rmu` to the top of the atmosphere.

@group(0) @binding(0) var<uniform> atm: Atmosphere;
@group(0) @binding(6) var t_out: texture_storage_2d<rgba16float, write>;

const TRANSMITTANCE_STEPS: u32 = 64u;

@compute @workgroup_size(8, 8, 1)
fn transmittance(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(t_out);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(size);
    let rmu = transmittance_uv_to_rmu(uv);
    let r = rmu.x;
    let mu = rmu.y;
    let dist = distance_to_top(r, mu);
    let dt = dist / f32(TRANSMITTANCE_STEPS);
    var optical = vec3<f32>(0.0);
    for (var i = 0u; i < TRANSMITTANCE_STEPS; i = i + 1u) {
        let t = (f32(i) + 0.5) * dt;
        let r_t = sqrt(max(r * r + 2.0 * r * mu * t + t * t, 0.0));
        optical = optical + medium_at(r_t).extinction * dt;
    }
    textureStore(t_out, vec2<i32>(id.xy), vec4<f32>(exp(-optical), 1.0));
}

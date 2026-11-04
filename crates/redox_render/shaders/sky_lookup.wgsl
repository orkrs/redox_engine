// Texture lookups for the atmosphere LUTs. Include after `sky_common.wgsl`.
//
// The including shader declares the globals used here:
//   transmittance_lookup  needs `t_transmittance` and `s_lut`
//   multiscatter_lookup   needs `t_multiscatter` and `s_lut`
//   sky_view_lookup       needs `t_sky_view` and `s_sky`
// A shader includes only the lookups it calls.

// Transmittance from radius r along the ray with cosine mu, up to the top of the atmosphere.
fn transmittance_lookup(r: f32, mu: f32) -> vec3<f32> {
    if (hits_ground(r, mu)) {
        return vec3<f32>(0.0);
    }
    let uv = transmittance_rmu_to_uv(r, mu);
    return textureSampleLevel(t_transmittance, s_lut, uv, 0.0).rgb;
}

// Multiple-scattering luminance psi (per unit sun irradiance) at radius r and sun cosine mu_s.
fn multiscatter_lookup(r: f32, mu_s: f32) -> vec3<f32> {
    let uv = multiscatter_rmu_to_uv(r, mu_s);
    return textureSampleLevel(t_multiscatter, s_lut, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
}

// Sky radiance seen along unit direction d from radius r (includes the ground).
fn sky_view_lookup(d: vec3<f32>, r: f32) -> vec3<f32> {
    let uv = sky_view_uv_from_dir(d, r);
    return textureSampleLevel(t_sky_view, s_sky, uv, 0.0).rgb;
}

// Sky-view LUT (Hillaire 2020, section 5.3). Rust prepends `sky_common.wgsl` and
// `sky_lookup.wgsl`. Rebuilt every frame: it depends on the sun direction and on the camera
// radius (`atm.view.x`). Output is radiance in nits, sun irradiance included.

@group(0) @binding(0) var<uniform> atm: Atmosphere;
@group(0) @binding(1) var t_transmittance: texture_2d<f32>;
@group(0) @binding(2) var s_lut: sampler;
@group(0) @binding(3) var t_multiscatter: texture_2d<f32>;
@group(0) @binding(4) var t_sky_view: texture_2d<f32>;
@group(0) @binding(5) var s_sky: sampler;
@group(0) @binding(6) var t_out: texture_storage_2d<rgba16float, write>;

const SKY_STEPS: u32 = 30u;

@compute @workgroup_size(8, 8, 1)
fn sky_view(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(t_out);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(size);
    let r = atm.view.x;
    let dir = sky_view_dir_from_uv(uv, r);
    let origin = vec3<f32>(0.0, r, 0.0);
    let mu = dir.y;
    let sun = atm.sun_dir.xyz;
    let cos_theta = dot(dir, sun);
    let phase_r = phase_rayleigh(cos_theta);
    let phase_m = phase_mie(cos_theta);

    var dist = distance_to_top(r, mu);
    let ground = hits_ground(r, mu);
    if (ground) {
        dist = distance_to_ground(r, mu);
    }
    let dt = dist / f32(SKY_STEPS);
    var throughput = vec3<f32>(1.0);
    var luminance = vec3<f32>(0.0);
    for (var i = 0u; i < SKY_STEPS; i = i + 1u) {
        let t = (f32(i) + 0.5) * dt;
        let p = origin + dir * t;
        let r_p = length(p);
        let medium = medium_at(r_p);
        let mu_s = dot(p / r_p, sun);
        let ts = transmittance_lookup(r_p, mu_s);
        let ms = multiscatter_lookup(r_p, mu_s);
        // Single scattering weighted by the phase function, plus isotropic multiple scattering.
        let source = ts * (medium.scatter_r * phase_r + medium.scatter_m * phase_m)
            + ms * (medium.scatter_r + medium.scatter_m);
        luminance = luminance + throughput * source * segment_factor(medium.extinction, dt);
        throughput = throughput * exp(-medium.extinction * dt);
    }
    if (ground) {
        // Lambertian ground lit by the sun, seen through the atmosphere.
        let p = origin + dir * dist;
        let n = normalize(p);
        let mu_s = dot(n, sun);
        let ground_radiance = atm.ground_albedo.xyz / PI * transmittance_lookup(atm.radii.x, mu_s) * max(mu_s, 0.0);
        luminance = luminance + throughput * ground_radiance;
    }
    let irradiance = atm.sun_irradiance.xyz;
    textureStore(t_out, vec2<i32>(id.xy), vec4<f32>(luminance * irradiance, 1.0));
}

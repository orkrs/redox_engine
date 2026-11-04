// Multiple-scattering LUT (Hillaire 2020, section 5.2). Rust prepends `sky_common.wgsl`
// and `sky_lookup.wgsl`. Texel (x, y): x = sun zenith cosine, y = radius. Stores
// psi = L2 / (1 - f_ms), per unit sun irradiance.

@group(0) @binding(0) var<uniform> atm: Atmosphere;
@group(0) @binding(1) var t_transmittance: texture_2d<f32>;
@group(0) @binding(2) var s_lut: sampler;
@group(0) @binding(3) var t_multiscatter: texture_2d<f32>;
@group(0) @binding(4) var t_sky_view: texture_2d<f32>;
@group(0) @binding(5) var s_sky: sampler;
@group(0) @binding(6) var t_out: texture_storage_2d<rgba16float, write>;

const MS_DIRECTIONS: u32 = 8u;   // 8 x 8 = 64 directions, uniform on the sphere
const MS_STEPS: u32 = 20u;

// Integrates one ray from `origin` (on the +Y axis) in direction `dir`. Returns the
// second-order scattering (L2) and the scattered fraction (f_ms), both per unit irradiance.
fn ms_ray(origin: vec3<f32>, dir: vec3<f32>, sun: vec3<f32>) -> array<vec3<f32>, 2> {
    let r0 = length(origin);
    let mu = dot(origin / r0, dir);
    var dist = distance_to_top(r0, mu);
    if (hits_ground(r0, mu)) {
        dist = distance_to_ground(r0, mu);
    }
    let dt = dist / f32(MS_STEPS);
    var throughput = vec3<f32>(1.0);
    var l2 = vec3<f32>(0.0);
    var f_ms = vec3<f32>(0.0);
    for (var i = 0u; i < MS_STEPS; i = i + 1u) {
        let t = (f32(i) + 0.5) * dt;
        let p = origin + dir * t;
        let r = length(p);
        let medium = medium_at(r);
        let scatter = medium.scatter_r + medium.scatter_m;
        let mu_s = dot(p / r, sun);
        let seg = segment_factor(medium.extinction, dt);
        l2 = l2 + throughput * transmittance_lookup(r, mu_s) * scatter * seg;
        f_ms = f_ms + throughput * scatter * seg;
        throughput = throughput * exp(-medium.extinction * dt);
    }
    return array<vec3<f32>, 2>(l2, f_ms);
}

@compute @workgroup_size(8, 8, 1)
fn multiscatter(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(t_out);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(size);
    let rmu = multiscatter_uv_to_rmu(uv);
    let r = rmu.x;
    let mu_s = rmu.y;
    let origin = vec3<f32>(0.0, r, 0.0);
    let sun = vec3<f32>(sqrt(max(1.0 - mu_s * mu_s, 0.0)), mu_s, 0.0);

    var l2_sum = vec3<f32>(0.0);
    var f_sum = vec3<f32>(0.0);
    for (var i = 0u; i < MS_DIRECTIONS; i = i + 1u) {
        for (var j = 0u; j < MS_DIRECTIONS; j = j + 1u) {
            let cos_t = 1.0 - 2.0 * (f32(i) + 0.5) / f32(MS_DIRECTIONS);
            let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
            let phi = 2.0 * PI * (f32(j) + 0.5) / f32(MS_DIRECTIONS);
            let dir = vec3<f32>(sin_t * cos(phi), cos_t, sin_t * sin(phi));
            let res = ms_ray(origin, dir, sun);
            l2_sum = l2_sum + res[0];
            f_sum = f_sum + res[1];
        }
    }
    let n = f32(MS_DIRECTIONS * MS_DIRECTIONS);
    let l2 = l2_sum / n;
    let f_ms = f_sum / n;
    let psi = l2 / max(vec3<f32>(1.0) - f_ms, vec3<f32>(1e-4));
    textureStore(t_out, vec2<i32>(id.xy), vec4<f32>(psi, 1.0));
}

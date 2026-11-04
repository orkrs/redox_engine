// Procedural atmosphere, shared math (Hillaire 2020, "A Scalable and Production
// Ready Sky and Atmosphere Rendering Technique").
//
// No bindings here. Every shader that includes this file declares `atm` (binding 0)
// before it. Units: radii and heights in km, scattering coefficients per km, irradiance
// in lux, so radiance comes out in nits. Lookup helpers that sample textures live in
// `sky_lookup.wgsl`.
//
// The Atmosphere layout MUST match `AtmosphereUniform` in src/sky.rs.

const PI: f32 = 3.14159265358979;
const UP: vec3<f32> = vec3<f32>(0.0, 1.0, 0.0);

struct Atmosphere {
    // x planet radius, y top-of-atmosphere radius, z Rayleigh height, w Mie height (km)
    radii: vec4<f32>,
    // xyz Rayleigh scattering per km
    rayleigh: vec4<f32>,
    // xyz Mie scattering per km, w = Mie anisotropy g
    mie_scatter: vec4<f32>,
    // xyz Mie absorption per km
    mie_absorb: vec4<f32>,
    // xyz ozone absorption per km
    ozone_absorb: vec4<f32>,
    // xyz ground albedo
    ground_albedo: vec4<f32>,
    // xyz unit vector towards the sun
    sun_dir: vec4<f32>,
    // xyz sun illuminance (lux), w sun angular radius (rad)
    sun_irradiance: vec4<f32>,
    // x camera radius from the planet centre (km)
    view: vec4<f32>,
};

struct Medium {
    scatter_r: vec3<f32>,
    scatter_m: vec3<f32>,
    extinction: vec3<f32>,
};

// Scattering and extinction at radius r (km from the planet centre).
fn medium_at(r: f32) -> Medium {
    let h = r - atm.radii.x;
    let rho_r = exp(-h / atm.radii.z);
    let rho_m = exp(-h / atm.radii.w);
    // Ozone: tent profile, peak 1 at 25 km, zero at 10 km and 40 km.
    let rho_o = clamp(select(-h / 15.0 + 8.0 / 3.0, h / 15.0 - 2.0 / 3.0, h < 25.0), 0.0, 1.0);
    var m: Medium;
    m.scatter_r = atm.rayleigh.xyz * rho_r;
    m.scatter_m = atm.mie_scatter.xyz * rho_m;
    m.extinction = m.scatter_r + m.scatter_m + atm.mie_absorb.xyz * rho_m + atm.ozone_absorb.xyz * rho_o;
    return m;
}

// Optical segment factor (1 - exp(-ext dt)) / ext, the integral of a decaying source over a step.
fn segment_factor(ext: vec3<f32>, dt: f32) -> vec3<f32> {
    return (vec3<f32>(1.0) - exp(-ext * dt)) / max(ext, vec3<f32>(1e-9));
}

fn phase_rayleigh(cos_t: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + cos_t * cos_t);
}

fn phase_mie(cos_t: f32) -> f32 {
    let g = atm.mie_scatter.w;
    let g2 = g * g;
    let denom = pow(max(1.0 + g2 - 2.0 * g * cos_t, 1e-4), 1.5);
    return 3.0 / (8.0 * PI) * (1.0 - g2) * (1.0 + cos_t * cos_t) / ((2.0 + g2) * denom);
}

// Distance from radius r along a ray with cosine mu to the top of the atmosphere.
fn distance_to_top(r: f32, mu: f32) -> f32 {
    let rt = atm.radii.y;
    return -r * mu + sqrt(max(rt * rt - r * r + r * r * mu * mu, 0.0));
}

// True when the ray from radius r with cosine mu hits the ground.
fn hits_ground(r: f32, mu: f32) -> bool {
    let rg = atm.radii.x;
    return mu < 0.0 && r * r * (1.0 - mu * mu) < rg * rg;
}

// Distance to the ground along a ray that hits it.
fn distance_to_ground(r: f32, mu: f32) -> f32 {
    let rg = atm.radii.x;
    return -r * mu - sqrt(max(r * r * (mu * mu - 1.0) + rg * rg, 0.0));
}

// Transmittance LUT (Bruneton-style): u = distance to the top, v = radius.
// uv -> (r, mu).
fn transmittance_uv_to_rmu(uv: vec2<f32>) -> vec2<f32> {
    let rg = atm.radii.x;
    let rt = atm.radii.y;
    let h = sqrt(rt * rt - rg * rg);
    let rho = h * uv.y;
    let r = sqrt(rho * rho + rg * rg);
    let d_min = rt - r;
    let d_max = rho + h;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if (d > 0.0) {
        mu = (h * h - rho * rho - d * d) / (2.0 * r * d);
    }
    return vec2<f32>(r, clamp(mu, -1.0, 1.0));
}

// (r, mu) -> uv, inverse of transmittance_uv_to_rmu.
fn transmittance_rmu_to_uv(r: f32, mu: f32) -> vec2<f32> {
    let rg = atm.radii.x;
    let rt = atm.radii.y;
    let h = sqrt(rt * rt - rg * rg);
    let rho = sqrt(max(r * r - rg * rg, 0.0));
    let d = distance_to_top(r, mu);
    let d_min = rt - r;
    let d_max = rho + h;
    return vec2<f32>((d - d_min) / max(d_max - d_min, 1e-6), rho / h);
}

// Multi-scattering LUT: u = cosine of the sun zenith, v = radius between ground and top.
fn multiscatter_uv_to_rmu(uv: vec2<f32>) -> vec2<f32> {
    let rg = atm.radii.x;
    let rt = atm.radii.y;
    return vec2<f32>(mix(rg, rt, uv.y), 2.0 * uv.x - 1.0);
}

fn multiscatter_rmu_to_uv(r: f32, mu_s: f32) -> vec2<f32> {
    let rg = atm.radii.x;
    let rt = atm.radii.y;
    return vec2<f32>(0.5 * mu_s + 0.5, (r - rg) / (rt - rg));
}

// Zenith angle of the ray that grazes the planet from radius r.
fn horizon_zenith(r: f32) -> f32 {
    let rg = atm.radii.x;
    return 0.5 * PI + acos(clamp(rg / r, -1.0, 1.0));
}

// Horizontal unit vector towards the sun. Falls back to +X when the sun is at the zenith.
fn sun_horizontal() -> vec3<f32> {
    let s = vec3<f32>(atm.sun_dir.x, 0.0, atm.sun_dir.z);
    if (dot(s, s) < 1e-8) {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    return normalize(s);
}

// Sky-view LUT: u = azimuth from the sun's horizontal direction (0..1 = 0..2pi),
// v = zenith parameter. Rays above the horizon fill v in [0, 0.5] densely near the horizon,
// rays into the ground fill v in [0.5, 1].
fn sky_view_dir_from_uv(uv: vec2<f32>, r: f32) -> vec3<f32> {
    let zh = horizon_zenith(r);
    var zenith: f32;
    if (uv.y <= 0.5) {
        let k = 1.0 - 2.0 * uv.y;
        zenith = zh * (1.0 - k * k);
    } else {
        zenith = zh + (PI - zh) * (2.0 * uv.y - 1.0);
    }
    let phi = uv.x * 2.0 * PI;
    let s = sun_horizontal();
    let t = cross(UP, s);
    return cos(zenith) * UP + sin(zenith) * (cos(phi) * s + sin(phi) * t);
}

// Inverse of sky_view_dir_from_uv for a unit direction d seen from radius r.
fn sky_view_uv_from_dir(d: vec3<f32>, r: f32) -> vec2<f32> {
    let zh = horizon_zenith(r);
    let zenith = acos(clamp(d.y, -1.0, 1.0));
    var v: f32;
    if (zenith <= zh) {
        v = 0.5 * (1.0 - sqrt(max(1.0 - zenith / zh, 0.0)));
    } else {
        v = 0.5 + 0.5 * (zenith - zh) / (PI - zh);
    }
    let s = sun_horizontal();
    let t = cross(UP, s);
    var phi = atan2(dot(d, t), dot(d, s));
    if (phi < 0.0) {
        phi = phi + 2.0 * PI;
    }
    return vec2<f32>(phi / (2.0 * PI), v);
}

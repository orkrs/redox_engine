struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) in_vertex_index: u32,
) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32((in_vertex_index << 1u) & 2u)) * 2.0 - 1.0;
    let y = f32(i32(in_vertex_index & 2u)) * 2.0 - 1.0;
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var t_hdr: texture_2d<f32>;
@group(0) @binding(1) var s_hdr: sampler;

struct TonemapUniform {
    exposure: f32,
    sharpen_enabled: u32,
    bloom_enabled: u32,
    rcas_sharpness: f32,
    output_size: vec2<f32>,
};

@group(0) @binding(2) var<uniform> tonemap_u: TonemapUniform;
@group(0) @binding(3) var t_bloom: texture_2d<f32>;

// True when the render target is an sRGB format: the hardware encodes on write, so the shader
// must output linear values. The curve below already yields display-referred (gamma-encoded)
// values, which on an sRGB target would be encoded a second time and look washed out.
override OUTPUT_SRGB: bool = false;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

/// Display-referred colour -> value to write to the target.
fn to_target(c: vec3<f32>) -> vec3<f32> {
    if (OUTPUT_SRGB) {
        return srgb_to_linear(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    return c;
}

const RCAS_LIMIT: f32 = 0.25 - (1.0 / 16.0);
const BLOOM_INTENSITY: f32 = 0.4;

// ACES kept for reference but not used — AgX is now the display transform.
fn aces_filmic(x: vec3<f32>) -> vec3<f32> {
    return clamp(
        (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
}

// AgX — Filmic AgX (Blender / Troy Sobotka) approximation, punchier than ACES,
// keeps highlight hue (no cyan shift), used as the sole display tonemapper.
fn agx_tonemap(x: vec3<f32>) -> vec3<f32> {
    var c = max(x, vec3<f32>(0.0));
    c = pow(c, vec3<f32>(0.96)); // was 0.85 → пастель; 0.96 сохраняет saturation
    let a = c * (6.2 * c + 0.5);
    let b = c * (6.2 * c + 1.7) + 0.06;
    return clamp(a / max(b, vec3<f32>(1e-5)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn pgrain(uv: vec2<f32>) -> f32 {
    // Interleaved gradient noise — stable, no tiling, cheap.
    return fract(sin(dot(uv * 413.0, vec2<f32>(17.13, 81.73))) * 43758.5453);
}

fn apply_film_optics(c: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    var out = c;
    // Subtle grain — 1% in midtones only, was 3% dusty.
    let g = pgrain(uv) - 0.5;
    let luma = dot(out, vec3<f32>(0.2126, 0.7152, 0.0722));
    let grain_w = 0.01 * clamp(1.0 - abs(luma - 0.5) * 1.7, 0.0, 1.0);
    out = out + g * grain_w;
    // Soft vignette 5% at corners, was 22% dark → пастель.
    let d2 = dot(uv - 0.5, uv - 0.5);
    let vign = pow(clamp(1.0 - d2 * 0.55, 0.0, 1.0), 2.0);
    let vign4 = vign * vign; // ≈ cos^4
    out = out * mix(0.95, 1.0, vign4);
    return clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn prep_agx(hdr: vec3<f32>) -> vec3<f32> {
    var c = hdr;
    if (any(c != c) || any(abs(c) > vec3<f32>(65504.0))) {
        c = vec3<f32>(0.0);
    }
    return agx_tonemap(c * tonemap_u.exposure);
}

fn prep_display(hdr: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    return apply_film_optics(prep_agx(hdr), uv);
}

fn rcas(b: vec3<f32>, d: vec3<f32>, e: vec3<f32>, f: vec3<f32>, h: vec3<f32>, sharpness_stops: f32) -> vec3<f32> {
    let bL = b.b * 0.5 + (b.r * 0.5 + b.g);
    let dL = d.b * 0.5 + (d.r * 0.5 + d.g);
    let eL = e.b * 0.5 + (e.r * 0.5 + e.g);
    let fL = f.b * 0.5 + (f.r * 0.5 + f.g);
    let hL = h.b * 0.5 + (h.r * 0.5 + h.g);

    let range = max(max(max(max(bL, dL), eL), fL), hL) - min(min(min(min(bL, dL), eL), fL), hL);
    var nz = 0.25 * bL + 0.25 * dL + 0.25 * fL + 0.25 * hL - eL;
    nz = clamp(abs(nz) / max(range, 1e-5), 0.0, 1.0);
    nz = 1.0 - 0.5 * nz;

    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));

    let hit_min = min(mn4, e) / max(mx4 * 4.0, vec3<f32>(1e-5));
    let hit_max = (vec3<f32>(1.0) - max(mx4, e)) / (mn4 * 4.0 - vec3<f32>(4.0));
    let lobe_rgb = max(-hit_min, hit_max);
    var lobe = max(lobe_rgb.x, max(lobe_rgb.y, lobe_rgb.z));
    lobe = max(-RCAS_LIMIT, min(lobe, 0.0)) * exp2(-sharpness_stops);
    lobe *= nz;

    let rcp_l = 1.0 / (4.0 * lobe + 1.0);
    return (lobe * (b + d + h + f) + e) * rcp_l;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / max(tonemap_u.output_size, vec2<f32>(1.0));
    var hdr = textureSample(t_hdr, s_hdr, in.uv).rgb;
    if (tonemap_u.bloom_enabled != 0u) {
        hdr += textureSample(t_bloom, s_hdr, in.uv).rgb * BLOOM_INTENSITY;
    }
    // AgX before RCAS, optics after — grain not sharpened.
    let e_agx = prep_agx(hdr);

    if (tonemap_u.sharpen_enabled == 0u) {
        return vec4<f32>(to_target(apply_film_optics(e_agx, in.uv)), 1.0);
    }

    var hdr_b = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(0.0, -texel.y)).rgb;
    var hdr_h = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(0.0, texel.y)).rgb;
    var hdr_d = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(-texel.x, 0.0)).rgb;
    var hdr_f = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(texel.x, 0.0)).rgb;
    if (tonemap_u.bloom_enabled != 0u) {
        let bloom_b = textureSample(t_bloom, s_hdr, in.uv + vec2<f32>(0.0, -texel.y)).rgb * BLOOM_INTENSITY;
        let bloom_h = textureSample(t_bloom, s_hdr, in.uv + vec2<f32>(0.0, texel.y)).rgb * BLOOM_INTENSITY;
        let bloom_d = textureSample(t_bloom, s_hdr, in.uv + vec2<f32>(-texel.x, 0.0)).rgb * BLOOM_INTENSITY;
        let bloom_f = textureSample(t_bloom, s_hdr, in.uv + vec2<f32>(texel.x, 0.0)).rgb * BLOOM_INTENSITY;
        hdr_b += bloom_b;
        hdr_h += bloom_h;
        hdr_d += bloom_d;
        hdr_f += bloom_f;
    }
    let b = prep_agx(hdr_b);
    let h = prep_agx(hdr_h);
    let d = prep_agx(hdr_d);
    let f = prep_agx(hdr_f);
    let sharpened = clamp(rcas(b, d, e_agx, f, h, tonemap_u.rcas_sharpness), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(to_target(apply_film_optics(sharpened, in.uv)), 1.0);
}
// Display-referred RCAS-only pass: input is already tonemapped LDR.
// Adds grain+vignette after RCAS so Sharp AA path also gets film optics.
@fragment
fn fs_present_rcas(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0 / max(tonemap_u.output_size, vec2<f32>(1.0));
    let e_raw = textureSample(t_hdr, s_hdr, in.uv).rgb;
    let e = apply_film_optics(e_raw, in.uv);

    if (tonemap_u.sharpen_enabled == 0u) {
        return vec4<f32>(to_target(e), 1.0);
    }

    let b_raw = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(0.0, -texel.y)).rgb;
    let h_raw = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(0.0, texel.y)).rgb;
    let d_raw = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(-texel.x, 0.0)).rgb;
    let f_raw = textureSample(t_hdr, s_hdr, in.uv + vec2<f32>(texel.x, 0.0)).rgb;
    let b = apply_film_optics(b_raw, in.uv + vec2<f32>(0.0, -texel.y));
    let h = apply_film_optics(h_raw, in.uv + vec2<f32>(0.0, texel.y));
    let d = apply_film_optics(d_raw, in.uv + vec2<f32>(-texel.x, 0.0));
    let f = apply_film_optics(f_raw, in.uv + vec2<f32>(texel.x, 0.0));
    return vec4<f32>(
        to_target(clamp(rcas(b, d, e, f, h, tonemap_u.rcas_sharpness), vec3<f32>(0.0), vec3<f32>(1.0))),
        1.0,
    );
}

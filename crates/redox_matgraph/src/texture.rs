//! Texture helpers (MAT-03 step 1): normal-map decoding, triplanar weights, and the WGSL text
//! of the helper functions that the generated code calls.
//!
//! The generated code expects these bindings in scope (the renderer declares them):
//! `tex<slot>: texture_2d<f32>` for each texture slot, and the samplers `samp_default`,
//! `samp_repeat` and `samp_clamp`.

/// Decodes a tangent-space normal texel (`rgba`, values in `[0, 1]`) and rotates it into world
/// space with the surface normal `n` and tangent `t`. Matches the WGSL `mg_normal_map`.
pub fn normal_map(texel: [f32; 4], n: [f32; 3], t: [f32; 3]) -> [f32; 3] {
    let cx = texel[0] * 2.0 - 1.0;
    let cy = texel[1] * 2.0 - 1.0;
    let cz = (1.0 - (cx * cx + cy * cy)).max(0.0).sqrt();
    let nn = normalize(n);
    let tg = normalize(t);
    let bt = cross(nn, tg);
    normalize([
        tg[0] * cx + bt[0] * cy + nn[0] * cz,
        tg[1] * cx + bt[1] * cy + nn[1] * cz,
        tg[2] * cx + bt[2] * cy + nn[2] * cz,
    ])
}

/// Triplanar blend weights of normal `n`: `|n_i|^4` normalised to sum to 1 (the sum is clamped
/// by `1e-6` so a zero normal gives zero weights instead of NaN). Matches `mg_triplanar`.
pub fn triplanar_weights(n: [f32; 3]) -> [f32; 3] {
    let w = [
        n[0].abs().powi(4),
        n[1].abs().powi(4),
        n[2].abs().powi(4),
    ];
    let sum = (w[0] + w[1] + w[2]).max(1e-6);
    [w[0] / sum, w[1] / sum, w[2] / sum]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = dot(v, v).sqrt();
    if len > 0.0 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        v
    }
}

/// WGSL helpers `mg_normal_map` and `mg_triplanar`. Emitted by the code generator when a
/// graph uses a texture node.
pub const WGSL: &str = r#"
fn mg_normal_map(tex: texture_2d<f32>, s: sampler, uv: vec2<f32>, n: vec3<f32>, t: vec3<f32>) -> vec3<f32> {
    let c = textureSample(tex, s, uv).xy * 2.0 - 1.0;
    let tn = vec3<f32>(c, sqrt(max(0.0, 1.0 - dot(c, c))));
    let nn = normalize(n);
    let tg = normalize(t);
    let bt = cross(nn, tg);
    return normalize(tg * tn.x + bt * tn.y + nn * tn.z);
}

fn mg_triplanar(tex: texture_2d<f32>, s: sampler, pos: vec3<f32>, n: vec3<f32>, scale: f32) -> vec4<f32> {
    var w = pow(abs(n), vec3<f32>(4.0));
    w = w / max(w.x + w.y + w.z, 1e-6);
    let cx = textureSample(tex, s, pos.zy * scale);
    let cy = textureSample(tex, s, pos.xz * scale);
    let cz = textureSample(tex, s, pos.xy * scale);
    return cx * w.x + cy * w.y + cz * w.z;
}
"#;

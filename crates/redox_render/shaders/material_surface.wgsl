// ── Shared material surface (MAT-01) ─────────────────────────────────────
//
// Prepended to `pbr.wgsl` by `PBR_WGSL` (see shader/mod.rs). WGSL has no
// includes, so this file holds only TYPES and PURE FUNCTIONS: it declares no
// bindings. Each pass reads its textures itself (`material_sample_textures`
// in pbr.wgsl) and passes the samples in as `TextureSamples`.

/// Layout identical to `material::MaterialUniform` (group 2 uniform).
struct Material {
    base_color: vec4<f32>,
    emissive:   vec4<f32>,
    metallic:   f32,
    roughness:  f32,
    flags:      u32,
    sheen:      f32,
    // Substrate lobes (layer0/1): rgb + metallic; params = roughness, weight, pad, pad.
    layer0_color: vec4<f32>,
    layer0_params: vec4<f32>,
    layer1_color: vec4<f32>,
    layer1_params: vec4<f32>,
    // MAT-04/05 rows (appended, 176-byte uniform): coat (weight, roughness, ior, aniso),
    // coat tint, subsurface colour (rgb, radius mm), subsurface params (wrap, ...).
    coat_params: vec4<f32>,
    coat_tint: vec4<f32>,
    sss_color: vec4<f32>,
    sss_params: vec4<f32>,
};

/// Per-fragment inputs of the material surface function.
struct SurfaceInput {
    world_pos: vec3<f32>,
    world_normal: vec3<f32>,
    world_tangent: vec3<f32>,
    instance_color: vec4<f32>,
    /// Camera position minus fragment position (not normalized).
    to_camera: vec3<f32>,
};

/// Texture reads of one fragment, sampled by the pass before `material_surface`.
/// Each field is only meaningful when the matching `Material.flags` bit is set;
/// `dither` is the blue-noise threshold for stochastic alpha.
struct TextureSamples {
    albedo: vec4<f32>,
    normal: vec4<f32>,
    mr: vec4<f32>,
    dither: f32,
};

/// Everything the lighting code reads from a material surface.
struct SurfaceData {
    base_color: vec3<f32>,
    opacity: f32,
    normal: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
    // Substrate lobes: layer rgb + metallic, then params (roughness, weight, pad, pad).
    layer0: vec4<f32>,
    layer0_params: vec4<f32>,
    layer1: vec4<f32>,
    layer1_params: vec4<f32>,
    sheen: f32,
};

/// Material surface of one fragment: albedo and alpha, alpha test, shading normal and
/// roughness. The body is the former `fs_main` prologue moved verbatim (same operation order).
/// May `discard` (stochastic alpha), as the original prologue did.
fn material_surface(in: SurfaceInput, mat: Material, tex: TextureSamples) -> SurfaceData {
    var albedo = mat.base_color.rgb * in.instance_color.rgb;
    var alpha = mat.base_color.a * in.instance_color.a;
    if ((mat.flags & 1u) != 0u) {
        albedo *= tex.albedo.rgb;
        alpha *= tex.albedo.a;
    }

    // Stochastic alpha-to-coverage with blue noise + golden-ratio temporal offset.
    let use_alpha_test = (mat.flags & 16u) != 0u;
    if (use_alpha_test || alpha < 0.999) {
        if (alpha < tex.dither) {
            discard;
        }
    }

    var n = in.world_normal;
    let n_len_sq = dot(n, n);
    if (n_len_sq > 1.0e-12) {
        n = n * inverseSqrt(n_len_sq);
    } else {
        n = vec3<f32>(0.0, 1.0, 0.0);
    }
    var toksvig_delta = 0.0;
    if ((mat.flags & 2u) != 0u) {
        let t = normalize(in.world_tangent);
        let b = cross(n, t);
        let tbn = mat3x3<f32>(t, b, n);
        let normal_sample = tex.normal;
        let normal_map = normal_sample.xyz * 2.0 - 1.0;
        // Alpha of Toksvig-baked normal mips stores roughness delta in [0,1]
        // mapped to [0, 0.5] (see resource/toksvig.rs).
        toksvig_delta = normal_sample.a * 0.5;
        n = normalize(tbn * normal_map);
    }

    // Geometric normal of the visible face. If the interpolated/CPU normal
    // points into the mesh, lighting looks sun-side-dark / shade-side-bright.
    // `normalize(cross(0))` is NaN and poisons the whole HDR buffer via bloom/TAA.
    let d_cross = cross(dpdx(in.world_pos), dpdy(in.world_pos));
    let d_len_sq = dot(d_cross, d_cross);
    if (d_len_sq > 1.0e-8) {
        var n_geo = d_cross * inverseSqrt(d_len_sq);
        if (dot(n_geo, in.to_camera) < 0.0) {
            n_geo = -n_geo;
        }
        // Only flip when clearly inverted — soft threshold avoids diagonal
        // triangulation seams (dpdx/dpdy jumps across the quad split).
        if (dot(n, n_geo) < -0.55) {
            n = -n;
        }
    }

    var metallic = mat.metallic;
    var roughness = mat.roughness;
    if ((mat.flags & 4u) != 0u) {
        let mr = tex.mr;
        metallic *= mr.b;
        roughness *= mr.g;
    }
    roughness = clamp(roughness + toksvig_delta, 0.05, 1.0);

    return SurfaceData(
        albedo,
        alpha,
        n,
        metallic,
        roughness,
        mat.emissive.rgb,
        mat.layer0_color,
        mat.layer0_params,
        mat.layer1_color,
        mat.layer1_params,
        mat.sheen,
    );
}

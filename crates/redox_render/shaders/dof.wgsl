// Depth of field: a disc blur whose radius is the circle of confusion of each pixel.
// Mirrors `linear_distance`, `coc_pixels` and `disc_taps` in src/post/post_fx.rs; keep in sync.
// Standard depth convention: 0 at the near plane, 1 at the far plane.

struct DofUniform {
    screen     : vec2<f32>,
    near       : f32,
    far        : f32,
    focus      : f32,
    aperture   : f32,
    max_radius : f32,
    taps       : u32,
};

const GOLDEN_ANGLE : f32 = 2.39996323;

@group(0) @binding(0) var t_color  : texture_2d<f32>;
@group(0) @binding(1) var t_depth  : texture_depth_2d;
@group(0) @binding(2) var s_linear : sampler;
@group(0) @binding(3) var<uniform>   dof : DofUniform;

struct VsOut {
    @builtin(position) clip_pos : vec4<f32>,
    @location(0)       uv       : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    out.uv       = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

fn linear_distance(depth: f32) -> f32 {
    return dof.near * dof.far / (dof.far - depth * (dof.far - dof.near));
}

fn coc_pixels(distance_w: f32) -> f32 {
    if (!(distance_w > 0.0) || distance_w > 3.0e38) {
        return 0.0;
    }
    let coc = dof.aperture * abs(distance_w - dof.focus) / distance_w;
    return clamp(coc, 0.0, dof.max_radius);
}

// Linear distance at a UV, read with textureLoad so the depth buffer can differ in size.
fn depth_distance_at(uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(t_depth));
    let px = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    return linear_distance(textureLoad(t_depth, px, 0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let center = textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    let coc = coc_pixels(depth_distance_at(in.uv));
    if (coc < 0.5) {
        return center;
    }
    let n = dof.taps;
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let r = select(0.0, sqrt(f32(i) / f32(n - 1u)), n > 1u);
        let a = f32(i) * GOLDEN_ANGLE;
        let off_px = vec2<f32>(r * cos(a), r * sin(a)) * coc;
        let uv_t = in.uv + off_px / dof.screen;
        // A tap that is sharper than its own distance from the centre lies in front of the
        // focus plane: give it zero weight so sharp foreground does not bleed into the blur.
        let tap_coc = coc_pixels(depth_distance_at(uv_t));
        let w = select(1.0, 0.0, tap_coc < length(off_px));
        sum = sum + textureSampleLevel(t_color, s_linear, uv_t, 0.0) * w;
        wsum = wsum + w;
    }
    if (wsum <= 0.0) {
        return center;
    }
    return sum / wsum;
}

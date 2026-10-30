// Blends the GI-cache reflection (`reflect.a` = weight, `reflect.rgb` = radiance) over the scene
// colour (GI-04). Runs in the post chain before SSR, which overrides it where it hits.
@group(0) @binding(0) var t_color   : texture_2d<f32>;
@group(0) @binding(1) var t_reflect : texture_2d<f32>;
@group(0) @binding(2) var s_linear  : sampler;
@group(0) @binding(3) var<uniform>   unused : vec4<f32>;

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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(t_color, s_linear, in.uv, 0.0);
    let dims = vec2<i32>(textureDimensions(t_reflect));
    let px = clamp(vec2<i32>(in.uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    let r = textureLoad(t_reflect, px, 0);
    return vec4<f32>(mix(color.rgb, r.rgb, r.a), color.a + unused.x * 0.0);
}

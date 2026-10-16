struct MsaaResolveUniform {
    sample_count : u32,
};

@group(0) @binding(0) var t_msaa : texture_multisampled_2d<f32>;
@group(0) @binding(1) var<uniform> msaa_u : MsaaResolveUniform;

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

fn luma_weight(rgb: vec3<f32>) -> f32 {
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return 1.0 / (1.0 + luma);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = textureDimensions(t_msaa);
    let coord = vec2<i32>(
        i32(in.uv.x * f32(dims.x)),
        i32(in.uv.y * f32(dims.y)),
    );

    var accum = vec3<f32>(0.0);
    var weight_sum = 0.0;

    // textureLoad requires a constant sample index on multisampled textures.
    {
        let rgb = textureLoad(t_msaa, coord, 0).rgb;
        let w = luma_weight(rgb);
        accum += rgb * w;
        weight_sum += w;
    }
    if (msaa_u.sample_count > 1u) {
        let rgb = textureLoad(t_msaa, coord, 1).rgb;
        let w = luma_weight(rgb);
        accum += rgb * w;
        weight_sum += w;
    }
    if (msaa_u.sample_count > 2u) {
        let rgb = textureLoad(t_msaa, coord, 2).rgb;
        let w = luma_weight(rgb);
        accum += rgb * w;
        weight_sum += w;
    }
    if (msaa_u.sample_count > 3u) {
        let rgb = textureLoad(t_msaa, coord, 3).rgb;
        let w = luma_weight(rgb);
        accum += rgb * w;
        weight_sum += w;
    }

    let resolved = accum / max(weight_sum, 1e-6);
    return vec4<f32>(resolved, 1.0);
}

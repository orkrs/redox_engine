// Hi-Z downsample: mip i → mip i+1 via 2×2 max depth reduction.

struct DownsampleParams {
    src_size: vec2<u32>,
    _pad: vec2<u32>,
};

@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var hiz_out: texture_storage_2d<r32float, write>;
@group(0) @binding(2) var<uniform> params: DownsampleParams;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dst_dims = textureDimensions(hiz_out);
    if (gid.x >= dst_dims.x || gid.y >= dst_dims.y) {
        return;
    }

    let src_coord = gid.xy * 2u;
    var max_d = 0.0;
    for (var oy = 0u; oy < 2u; oy++) {
        for (var ox = 0u; ox < 2u; ox++) {
            let sc = src_coord + vec2<u32>(ox, oy);
            if (sc.x < params.src_size.x && sc.y < params.src_size.y) {
                max_d = max(max_d, textureLoad(t_src, vec2<i32>(sc), 0).x);
            }
        }
    }
    textureStore(hiz_out, vec2<i32>(gid.xy), vec4<f32>(max_d, 0.0, 0.0, 0.0));
}

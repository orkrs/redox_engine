// Copy depth buffer (non-storage) into Hi-Z mip 0 (r32float storage).

@group(0) @binding(0) var t_depth: texture_depth_2d;
@group(0) @binding(1) var hiz_out: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(t_depth);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let coord = vec2<i32>(i32(gid.x), i32(gid.y));
    let d = textureLoad(t_depth, coord, 0);
    textureStore(hiz_out, coord, vec4<f32>(d, 0.0, 0.0, 0.0));
}

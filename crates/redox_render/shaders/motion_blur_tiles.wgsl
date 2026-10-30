// Velocity tile maxima for motion blur spreading (PST-03). Mirrors `tile_max` and
// `neighbor_max` in src/post/motion_blur_tiles.rs; keep them in sync. Velocity is stored in
// pixels: the UV offset per frame times the texture size. Both passes share one bind group
// layout: binding 0 is the source texture, binding 1 the destination tile texture.

const TILE: u32 = 20u;

@group(0) @binding(0) var t_src : texture_2d<f32>;
@group(0) @binding(1) var t_dst : texture_storage_2d<rgba16float, write>;

// Keeps `a` on ties, so the first velocity wins as in the Rust version.
fn longer(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    if (dot(b, b) > dot(a, a)) {
        return b;
    }
    return a;
}

// One invocation per tile: the longest velocity among the tile's pixels.
@compute @workgroup_size(8, 8, 1)
fn tile_max_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let tiles = textureDimensions(t_dst);
    if (gid.x >= tiles.x || gid.y >= tiles.y) {
        return;
    }
    let src_dims = textureDimensions(t_src);
    let dims_f = vec2<f32>(src_dims);
    let x0 = gid.x * TILE;
    let y0 = gid.y * TILE;
    let x1 = min(x0 + TILE, src_dims.x);
    let y1 = min(y0 + TILE, src_dims.y);
    var best = vec2<f32>(0.0);
    for (var y = y0; y < y1; y = y + 1u) {
        for (var x = x0; x < x1; x = x + 1u) {
            let uv_vel = textureLoad(t_src, vec2<i32>(i32(x), i32(y)), 0).xy;
            best = longer(best, uv_vel * dims_f);
        }
    }
    textureStore(t_dst, vec2<i32>(gid.xy), vec4<f32>(best, 0.0, 1.0));
}

// One invocation per tile: the longest tile maximum over its 3x3 neighbourhood.
@compute @workgroup_size(8, 8, 1)
fn neighbor_max_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let tiles = vec2<i32>(textureDimensions(t_dst));
    let c = vec2<i32>(gid.xy);
    if (c.x >= tiles.x || c.y >= tiles.y) {
        return;
    }
    var best = vec2<f32>(0.0);
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let p = c + vec2<i32>(dx, dy);
            if (p.x < 0 || p.y < 0 || p.x >= tiles.x || p.y >= tiles.y) {
                continue;
            }
            best = longer(best, textureLoad(t_src, p, 0).xy);
        }
    }
    textureStore(t_dst, c, vec4<f32>(best, 0.0, 1.0));
}

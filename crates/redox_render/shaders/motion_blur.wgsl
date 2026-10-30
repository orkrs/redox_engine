// Motion blur over the TAA output. Mirrors `motion_blur_offsets` in src/post/post_fx.rs and
// `spread_weight` in src/post/motion_blur_tiles.rs; keep them in sync. Velocity is read at the
// pixel (UV offset per frame). With `spread` on, a pixel that does not move itself also takes
// the blur of the fast tile next to it (PST-03), weighted by the depth test.

struct MotionBlurUniform {
    screen     : vec2<f32>,
    shutter    : f32,
    max_pixels : f32,
    taps       : u32,
    spread     : u32,
    near       : f32,
    far        : f32,
    soft_z     : f32,
    _pad0      : u32,
};

@group(0) @binding(0) var t_color    : texture_2d<f32>;
@group(0) @binding(1) var t_velocity : texture_2d<f32>;
@group(0) @binding(2) var t_depth    : texture_depth_2d;
@group(0) @binding(3) var t_neighbor : texture_2d<f32>;
@group(0) @binding(4) var s_linear   : sampler;
@group(0) @binding(5) var<uniform>     mb : MotionBlurUniform;

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

// Blur along the pixel's own velocity (the pass before PST-03).
fn own_blur(uv: vec2<f32>) -> vec4<f32> {
    let center = textureSampleLevel(t_color, s_linear, uv, 0.0);
    let vel = textureSampleLevel(t_velocity, s_linear, uv, 0.0).xy;
    let v0 = vel * mb.shutter;
    let len_px = length(v0 * mb.screen);
    // Same rule as the Rust side: blurs under half a pixel leave the pixel sharp.
    if (len_px < 0.5) {
        return center;
    }
    var v = v0;
    if (len_px > mb.max_pixels) {
        v = v0 * (mb.max_pixels / len_px);
    }
    let n = mb.taps;
    var sum = vec4<f32>(0.0);
    for (var i = 0u; i < n; i = i + 1u) {
        let t = f32(i) / f32(n - 1u) - 0.5;
        sum = sum + textureSampleLevel(t_color, s_linear, uv + v * t, 0.0);
    }
    return sum / f32(n);
}

// Linear view distance of a depth-buffer value (same formula as `linear_distance` in Rust).
fn linear_depth(z: f32) -> f32 {
    return mb.near * mb.far / (mb.far - z * (mb.far - mb.near));
}

// Same as `spread_weight` in src/post/motion_blur_tiles.rs. `neighbor_velocity` is in pixels.
fn spread_weight(neighbor_velocity: vec2<f32>, dist_pixels: f32, depth_p: f32, depth_neighbor: f32) -> f32 {
    let reach = length(neighbor_velocity) * mb.shutter;
    if (!(reach > 0.0) || !(reach >= dist_pixels)) {
        return 0.0;
    }
    if (!(depth_neighbor <= depth_p + mb.soft_z)) {
        return 0.0;
    }
    let t = dist_pixels / reach;
    let u = clamp((t - 0.5) / 0.5, 0.0, 1.0);
    return 1.0 - u * u * (3.0 - 2.0 * u);
}

// Tile index of a pixel, clamped to the tile grid. Tiles are 20 px (TILE_SIZE in Rust).
fn tile_of(px: vec2<i32>) -> vec2<i32> {
    let dims = vec2<i32>(textureDimensions(t_neighbor));
    return clamp(px / vec2<i32>(20, 20), vec2<i32>(0), dims - vec2<i32>(1));
}

// Blends in the colour of the neighbouring tile's blur. Taps run from the pixel back along the
// neighbour's motion, out to its full blur length. Each tap is weighted by how far the tap's
// own tile reaches the pixel and by the depth test; the result is mixed over `own` by the
// total weight.
fn spread_blur(pos: vec2<f32>, own: vec4<f32>) -> vec4<f32> {
    let depth_dims = vec2<i32>(textureDimensions(t_depth));
    let pix = clamp(vec2<i32>(floor(pos)), vec2<i32>(0), depth_dims - vec2<i32>(1));
    let depth_p = linear_depth(textureLoad(t_depth, pix, 0));
    let vnb = textureLoad(t_neighbor, tile_of(pix), 0).xy;
    if (length(vnb) * mb.shutter < 0.5) {
        return own;
    }
    let n = mb.taps;
    var acc = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let s = f32(i) / f32(n - 1u);
        let offset = vnb * mb.shutter * s;
        let q = pos - offset;
        let qpx = clamp(vec2<i32>(floor(q)), vec2<i32>(0), depth_dims - vec2<i32>(1));
        let vq = textureLoad(t_neighbor, tile_of(qpx), 0).xy;
        let depth_q = linear_depth(textureLoad(t_depth, qpx, 0));
        let w = spread_weight(vq, length(offset), depth_p, depth_q);
        if (w > 0.0) {
            acc = acc + w * textureSampleLevel(t_color, s_linear, q / mb.screen, 0.0).rgb;
            wsum = wsum + w;
        }
    }
    if (wsum <= 0.0) {
        return own;
    }
    let alpha = clamp(wsum / f32(n), 0.0, 1.0);
    return mix(own, vec4<f32>(acc / wsum, 1.0), alpha);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let own = own_blur(in.uv);
    if (mb.spread == 0u) {
        return own;
    }
    return spread_blur(in.clip_pos.xy, own);
}

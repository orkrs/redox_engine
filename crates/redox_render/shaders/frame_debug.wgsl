// Debug composite for a black frame. Draws one target, or four quadrants:
//   top-left  red    — HDR after the resolve, before SSR / fog / motion blur
//   top-right green  — camera depth (white = far / nothing written)
//   bottom-left blue — world normals from the prepass
//   bottom-right yellow — HDR after the post chain
// A colored square at the screen center means this pass ran. A black quadrant
// with that square means the target itself is empty.

struct DebugUniform {
    exposure : f32,
    mode     : u32,
    _pad0    : u32,
    _pad1    : u32,
}

@group(0) @binding(0) var t_hdr    : texture_2d<f32>;
@group(0) @binding(1) var t_post   : texture_2d<f32>;
@group(0) @binding(2) var t_normal : texture_2d<f32>;
@group(0) @binding(3) var t_depth  : texture_depth_2d;
@group(0) @binding(4) var<uniform> dbg : DebugUniform;
// What the post chain actually reads (TAA history, or HDR when TAA is off).
@group(0) @binding(5) var t_chain  : texture_2d<f32>;

struct VsOut {
    @builtin(position) clip_pos : vec4<f32>,
    @location(0)       uv       : vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    out.uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

fn tonemap(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(1.0) - exp(-rgb * max(dbg.exposure, 0.01));
}

fn load_rgb(tex: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    let dims = vec2<i32>(textureDimensions(tex));
    let px = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(tex, px, 0).rgb;
}

fn load_depth(uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(t_depth));
    let px = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(t_depth, px, 0);
}

// 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
fn quadrant(uv: vec2<f32>) -> u32 {
    let right = u32(uv.x >= 0.5);
    let bottom = u32(uv.y >= 0.5);
    return right + bottom * 2u;
}

fn marker_color(slot: u32) -> vec3<f32> {
    if (slot == 1u) {
        return vec3<f32>(0.15, 1.0, 0.25);
    }
    if (slot == 2u) {
        return vec3<f32>(0.25, 0.45, 1.0);
    }
    if (slot == 3u) {
        return vec3<f32>(1.0, 0.85, 0.15);
    }
    return vec3<f32>(1.0, 0.15, 0.1);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // mode 1 = split, mode 6 = chain. modes 2..5 = one target (HDR, depth, normal, post).
    var slot = dbg.mode - 2u;
    var sample_uv = in.uv;
    if (dbg.mode == 1u || dbg.mode == 6u) {
        slot = quadrant(in.uv);
        sample_uv = fract(in.uv * 2.0);
    }

    var rgb = vec3<f32>(0.0);
    if (dbg.mode == 6u) {
        // TL drawn HDR, TR what TAA/post reads, BL post output, BR depth lifted so near-1 values show.
        if (slot == 0u) {
            rgb = tonemap(load_rgb(t_hdr, sample_uv));
        } else if (slot == 1u) {
            rgb = tonemap(load_rgb(t_chain, sample_uv));
        } else if (slot == 2u) {
            rgb = tonemap(load_rgb(t_post, sample_uv));
        } else {
            let d = load_depth(sample_uv);
            rgb = vec3<f32>(clamp((1.0 - d) * 12.0, 0.0, 1.0));
        }
    } else if (slot == 0u) {
        rgb = tonemap(load_rgb(t_hdr, sample_uv));
    } else if (slot == 1u) {
        let d = load_depth(sample_uv);
        rgb = vec3<f32>(d);
    } else if (slot == 2u) {
        rgb = load_rgb(t_normal, sample_uv) * 0.5 + vec3<f32>(0.5);
    } else {
        rgb = tonemap(load_rgb(t_post, sample_uv));
    }

    let center = abs(in.uv - vec2<f32>(0.5));
    if (center.x < 0.015 && center.y < 0.015) {
        rgb = marker_color(slot);
    }
    return vec4<f32>(rgb, 1.0);
}

// Screen-driven virtual shadow map page requests.
//
// Every second pixel reconstructs its world position, picks the clipmap level whose texel is
// about one pixel wide at that distance, and sets the bit of the page that covers it. The CPU
// reads the bitfield back a frame or two later and backs exactly those pages with memory.

struct VsmLevelGpu {
    view_proj: mat4x4<f32>,
    world_half_extent: f32,
    page_table_offset: u32,
    pages_per_side: u32,
    texel_world: f32,
}

// Prefix of `VsmGpuInfo` (shadow::vsm_clipmap): everything up to and including `levels`.
struct VsmInfoHead {
    level_count: u32,
    page_px: u32,
    phys_page_count: u32,
    phys_pages_per_side: u32,
    ray_count: u32,
    samples_per_ray: u32,
    source_angle: f32,
    source_radius: f32,
    max_trace_dist: f32,
    _pad0: vec3<f32>,
    levels: array<VsmLevelGpu, 8>,
}

struct RequestParams {
    inv_view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    width: u32,
    height: u32,
    // World size of one pixel per unit of distance.
    pixel_angle: f32,
    _pad: f32,
}

@group(0) @binding(0) var t_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> params: RequestParams;
@group(0) @binding(2) var<storage, read> vsm: VsmInfoHead;
@group(0) @binding(3) var<storage, read_write> requested: array<atomic<u32>>;

fn mark_page(level: u32, uv: vec2<f32>) {
    let cm = vsm.levels[level];
    let pps = cm.pages_per_side;
    let page = min(vec2<u32>(uv * f32(pps)), vec2<u32>(pps - 1u));
    let flat_index = cm.page_table_offset + page.y * pps + page.x;
    atomicOr(&requested[flat_index / 32u], 1u << (flat_index % 32u));
}

@compute @workgroup_size(8, 8, 1)
fn cs_request(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = gid.xy * 2u;
    if (px.x >= params.width || px.y >= params.height) {
        return;
    }
    let depth = textureLoad(t_depth, vec2<i32>(px), 0);
    if (depth >= 1.0) {
        return;
    }

    let uv = (vec2<f32>(px) + vec2<f32>(0.5)) / vec2<f32>(f32(params.width), f32(params.height));
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world_h = params.inv_view_proj * ndc;
    let world = world_h.xyz / world_h.w;

    // Finest level whose texel is not much finer than one screen pixel here.
    let footprint = distance(world, params.cam_pos.xyz) * params.pixel_angle;
    let count = min(vsm.level_count, 8u);
    var wanted = count - 1u;
    for (var l = 0u; l < count; l++) {
        if (vsm.levels[l].texel_world >= footprint * 0.5) {
            wanted = l;
            break;
        }
    }

    // First level at or above `wanted` whose window actually contains the point.
    for (var l = wanted; l < count; l++) {
        let clip = vsm.levels[l].view_proj * vec4<f32>(world, 1.0);
        if (clip.w <= 1.0e-4) {
            continue;
        }
        let n = clip.xyz / clip.w;
        let page_uv = vec2<f32>(n.x * 0.5 + 0.5, 0.5 - n.y * 0.5);
        if (page_uv.x < 0.0 || page_uv.x >= 1.0 || page_uv.y < 0.0 || page_uv.y >= 1.0
            || n.z < 0.0 || n.z > 1.0) {
            continue;
        }
        mark_page(l, page_uv);
        break;
    }
}

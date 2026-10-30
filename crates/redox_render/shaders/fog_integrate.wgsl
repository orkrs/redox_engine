// Froxel fog integration (PST-06 step 3). One invocation per column: walks the slices front to
// back and stores the accumulated in-scatter (rgb) and transmittance (a) of each slice end.
// Mirrors fog_inject.wgsl and src/post/fog_froxel.rs; keep in sync.

struct FogUniform {
    color_density : vec4<f32>,
    falloff       : vec4<f32>,
    scatter       : vec4<f32>,   // z near, w far
    camera_pos    : vec4<f32>,
    sun_dir       : vec4<f32>,
    sun_color     : vec4<f32>,
    inv_view_proj : mat4x4<f32>,
};

const FROXEL_W: u32 = 160u;
const FROXEL_H: u32 = 90u;
const FROXEL_D: u32 = 64u;
const FROXEL_D_F: f32 = 64.0;
const FROXEL_MAX_DISTANCE: f32 = 120.0;

@group(0) @binding(0) var<uniform> fog : FogUniform;
@group(0) @binding(1) var vol_in : texture_3d<f32>;
@group(0) @binding(2) var vol_out : texture_storage_3d<rgba16float, write>;

fn froxel_far() -> f32 {
    return min(fog.scatter.w, FROXEL_MAX_DISTANCE);
}

fn slice_depth(i: f32) -> f32 {
    let near = fog.scatter.z;
    return near * pow(froxel_far() / near, i / FROXEL_D_F);
}

fn view_ray(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let a = fog.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let b = fog.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    return normalize(a.xyz / a.w - b.xyz / b.w);
}

@compute @workgroup_size(8, 8, 1)
fn cs_integrate(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= FROXEL_W || id.y >= FROXEL_H) {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(f32(FROXEL_W), f32(FROXEL_H));
    let ray = view_ray(uv);
    let fwd = view_ray(vec2<f32>(0.5, 0.5));
    let cos_t = max(dot(ray, fwd), 1e-4);

    var acc = vec3<f32>(0.0);
    var trans = 1.0;
    for (var k = 0u; k < FROXEL_D; k++) {
        let v = textureLoad(vol_in, vec3<i32>(i32(id.x), i32(id.y), i32(k)), 0);
        let sigma = v.a;
        // Slab length along the ray.
        let ds = (slice_depth(f32(k + 1u)) - slice_depth(f32(k))) / cos_t;
        let step_trans = exp(-sigma * ds);
        var step_scatter = v.rgb * ds;
        if (sigma > 1e-6) {
            step_scatter = v.rgb * (1.0 - step_trans) / sigma;
        }
        acc += trans * step_scatter;
        trans *= step_trans;
        textureStore(vol_out, vec3<i32>(i32(id.x), i32(id.y), i32(k)), vec4<f32>(acc, trans));
    }
}

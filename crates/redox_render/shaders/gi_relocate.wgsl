// Probe relocation (GI-01 step 5b). One invocation per atlas index: a probe whose lattice point
// lies inside geometry (global SDF distance below 0.1 spacing) is moved half a spacing along the
// SDF gradient, out of the surface. The offset is written to probe_offset[index]; the trace and
// the shading add it to the lattice position. Every frame the offsets are recomputed from the
// current grid, so nothing accumulates.
//
// CPU mirror of the grid layout: gi::grid_gpu::GiGridGpu; of the cell mapping: gi_trace.wgsl.

struct GiCascadeGpu {
    origin: vec4<f32>,   // xyz origin, w probe spacing
    dims: vec4<u32>,     // xyz probe counts, w atlas index of the first probe
    scroll: vec4<i32>,   // xyz lowest world cell of the window
}
struct GiGridGpu {
    cascades: array<GiCascadeGpu, 2>,
    probes_per_row: u32,
    enabled: u32,
    cascade_count: u32,
    _pad: u32,
}
struct SdfInfo {
    origin_voxel: vec4<f32>, // volume minimum corner (xyz), voxel size (w)
    dims: vec4<u32>,         // voxel counts (xyz)
};

@group(0) @binding(0) var<uniform> grid: GiGridGpu;
@group(0) @binding(1) var t_distance: texture_3d<f32>;
@group(0) @binding(2) var s_linear: sampler;
@group(0) @binding(3) var<uniform> sdf: SdfInfo;
@group(0) @binding(4) var<storage, read_write> probe_offset: array<vec4<f32>>;

// Probes closer to a surface than this fraction of their spacing are moved.
const RELOCATE_DISTANCE: f32 = 0.1;
// Distance moved out of the surface, in spacings.
const RELOCATE_STEP: f32 = 0.5;

fn sdf_uvw(p: vec3<f32>) -> vec3<f32> {
    let extent = vec3<f32>(sdf.dims.xyz) * sdf.origin_voxel.w;
    return (p - sdf.origin_voxel.xyz) / extent;
}

fn sdf_distance(p: vec3<f32>) -> f32 {
    return textureSampleLevel(t_distance, s_linear, sdf_uvw(p), 0.0).r;
}

// Central-difference gradient of the distance field, normalised.
fn sdf_gradient(p: vec3<f32>) -> vec3<f32> {
    let h = sdf.origin_voxel.w;
    let g = vec3<f32>(
        sdf_distance(p + vec3<f32>(h, 0.0, 0.0)) - sdf_distance(p - vec3<f32>(h, 0.0, 0.0)),
        sdf_distance(p + vec3<f32>(0.0, h, 0.0)) - sdf_distance(p - vec3<f32>(0.0, h, 0.0)),
        sdf_distance(p + vec3<f32>(0.0, 0.0, h)) - sdf_distance(p - vec3<f32>(0.0, 0.0, h)),
    );
    let len = length(g);
    if (len < 1e-8) {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return g / len;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    // Cascade 0 owns atlas indices [0, per), cascade 1 the next `per` (see GiGridGpu::new).
    let per = grid.cascades[0].dims.x * grid.cascades[0].dims.y * grid.cascades[0].dims.z;
    let total = per * grid.cascade_count;
    if (index >= total) {
        return;
    }
    let c = index / per;
    // `select` takes scalars and vectors only, so the cascade is picked with a branch.
    var casc = grid.cascades[0];
    if (c == 1u) {
        casc = grid.cascades[1];
    }
    let dims = vec3<i32>(casc.dims.xyz);
    let local = index - c * per;
    let slot = vec3<i32>(
        i32(local % casc.dims.x),
        i32((local / casc.dims.x) % casc.dims.y),
        i32(local / (casc.dims.x * casc.dims.y)),
    );
    // The world cell this ring slot holds in the current window (see gi_trace.wgsl main).
    let rel = slot - casc.scroll.xyz;
    let cell = casc.scroll.xyz + ((rel % dims) + dims) % dims;
    let spacing = casc.origin.w;
    let pos = casc.origin.xyz + vec3<f32>(cell) * spacing;

    var offset = vec3<f32>(0.0);
    if (sdf_distance(pos) < RELOCATE_DISTANCE * spacing) {
        offset = sdf_gradient(pos) * (RELOCATE_STEP * spacing);
    }
    probe_offset[index] = vec4<f32>(offset, 0.0);
}

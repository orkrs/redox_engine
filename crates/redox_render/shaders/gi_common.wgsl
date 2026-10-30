// Shared GI helpers (GI-02 step 3): previous-frame probe irradiance and surface-card projection.
//
// Concatenated in front of gi_trace.wgsl and gi_card_light.wgsl (see shader/mod.rs). The
// including shader declares these globals with the same names and bindings:
//   @group(0) var<uniform> prev_grid: GiGridGpu;
//   @group(0) var t_prev_irradiance: texture_2d<f32>;
//   @group(0) var<storage, read> probe_offset: array<vec4<f32>>;
//
// Card functions mirror src/gi/cards.rs (project_to_card, unproject_from_card, face_weights):
// keep them in sync. Face index order is CardFace::ALL: 0 +X, 1 -X, 2 +Y, 3 -Y, 4 +Z, 5 -Z.

const PI: f32 = 3.14159265;
// Irradiance tile geometry (CPU: gi::atlas::{IRRADIANCE_INNER, TILE_BORDER}).
const GI_INNER_IRR: f32 = 8.0;
const GI_STRIDE_IRR: u32 = 10u;
// Shading point pushed off the surface along the normal, in cascade spacings.
const BOUNCE_NORMAL_BIAS: f32 = 0.2;

// Mirror of pbr.wgsl GiCascadeGpu / GiGridGpu (src/gi/grid_gpu.rs), for the previous frame.
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

// One object's cards (src/gi/card_atlas.rs `CardObjectGpu`, 128 bytes). `rects[f]` is the atlas
// rectangle (x, y, w, h) of face f; a zero size means the face has no card.
struct CardObjectGpu {
    aabb_min: vec4<f32>,
    aabb_max: vec4<f32>,
    rects: array<vec4<u32>, 6>,
}

fn oct_encode(n: vec3<f32>) -> vec2<f32> {
    var v = n / max(abs(n.x) + abs(n.y) + abs(n.z), 1.0e-6);
    if (v.y < 0.0) {
        let ox = v.x;
        v.x = (1.0 - abs(v.z)) * select(-1.0, 1.0, ox >= 0.0);
        v.z = (1.0 - abs(ox)) * select(-1.0, 1.0, v.z >= 0.0);
    }
    return clamp(v.xz * 0.5 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
}

// Previous frame's irradiance E in the direction `dir` of atlas probe `probe`, bilinear inside
// its tile (the stored value is E^(1/5); see gi::atlas).
fn prev_tile_irradiance(probe: u32, dir: vec3<f32>) -> vec3<f32> {
    let ppr = prev_grid.probes_per_row;
    let tile = vec2<f32>(f32(probe % ppr), f32(probe / ppr)) * f32(GI_STRIDE_IRR);
    let p = tile + 1.0 + oct_encode(dir) * GI_INNER_IRR - 0.5;
    let i0 = floor(p);
    let f = p - i0;
    let c = vec2<i32>(i0);
    let a = textureLoad(t_prev_irradiance, c, 0).rgb;
    let b = textureLoad(t_prev_irradiance, c + vec2<i32>(1, 0), 0).rgb;
    let d = textureLoad(t_prev_irradiance, c + vec2<i32>(0, 1), 0).rgb;
    let e = textureLoad(t_prev_irradiance, c + vec2<i32>(1, 1), 0).rgb;
    let stored = mix(mix(a, b, f.x), mix(d, e, f.x), f.y);
    return pow(max(stored, vec3<f32>(0.0)), vec3<f32>(5.0));
}

// Trilinear blend of the eight probes around `p` in one cascade, without the visibility test
// (the bounce term is cheap by design). Returns (weighted E sum, total weight).
fn prev_sample_cascade(casc: GiCascadeGpu, p: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let spacing = casc.origin.w;
    let biased = p + n * (BOUNCE_NORMAL_BIAS * spacing);
    let g = (biased - casc.origin.xyz) / spacing;
    let base = floor(g);
    let frac = g - base;
    let base_cell = vec3<i32>(base);
    let scroll = casc.scroll.xyz;
    let dims = vec3<i32>(casc.dims.xyz);
    var accum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var dz = 0; dz < 2; dz = dz + 1) {
        for (var dy = 0; dy < 2; dy = dy + 1) {
            for (var dx = 0; dx < 2; dx = dx + 1) {
                let cell = base_cell + vec3<i32>(dx, dy, dz);
                let rel = cell - scroll;
                if (any(rel < vec3<i32>(0)) || any(rel >= dims)) {
                    continue;
                }
                let slot = ((cell % dims) + dims) % dims;
                let su = vec3<u32>(slot);
                let index = casc.dims.w + su.x + su.y * casc.dims.x + su.z * casc.dims.x * casc.dims.y;
                let probe_pos = casc.origin.xyz + vec3<f32>(cell) * spacing + probe_offset[index].xyz;
                let wx = select(frac.x, 1.0 - frac.x, dx == 0);
                let wy = select(frac.y, 1.0 - frac.y, dy == 0);
                let wz = select(frac.z, 1.0 - frac.z, dz == 0);
                // Backface term as in pbr.wgsl: probes behind the surface count less, not zero.
                let to_probe = probe_pos - p;
                let dir_to_probe = to_probe / max(length(to_probe), 1.0e-5);
                let facing = (dot(dir_to_probe, n) + 1.0) * 0.5;
                let w = wx * wy * wz * (facing * facing + 0.2);
                accum = accum + prev_tile_irradiance(index, n) * w;
                wsum = wsum + w;
            }
        }
    }
    return vec4<f32>(accum, wsum);
}

// True when `p` lies inside the world window of cascade `casc`.
fn prev_cascade_contains(casc: GiCascadeGpu, p: vec3<f32>) -> bool {
    let lo = casc.origin.xyz + vec3<f32>(casc.scroll.xyz) * casc.origin.w;
    let hi = lo + vec3<f32>(casc.dims.xyz) * casc.origin.w;
    return all(p >= lo) && all(p <= hi);
}

// Irradiance E of the previous frame at a point: the fine cascade when the point is in its
// window, else the coarse one. Zero when GI is off or the point is outside both.
fn prev_irradiance(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if (prev_grid.enabled == 0u) {
        return vec3<f32>(0.0);
    }
    let fine = prev_grid.cascades[0];
    if (prev_cascade_contains(fine, p)) {
        let s = prev_sample_cascade(fine, p, n);
        if (s.w > 1.0e-5) {
            return s.rgb / s.w;
        }
    }
    let coarse = prev_grid.cascades[1];
    if (prev_grid.cascade_count > 1u && prev_cascade_contains(coarse, p)) {
        let s = prev_sample_cascade(coarse, p, n);
        if (s.w > 1.0e-5) {
            return s.rgb / s.w;
        }
    }
    return vec3<f32>(0.0);
}

// Axes (u, v, depth) of a face, as in CardFace::axes.
fn card_axes(face: u32) -> vec3<u32> {
    switch (face) {
        case 0u, 1u: { return vec3<u32>(1u, 2u, 0u); }
        case 2u, 3u: { return vec3<u32>(0u, 2u, 1u); }
        default: { return vec3<u32>(0u, 1u, 2u); }
    }
}

// Outward normal of a face, as in CardFace::normal.
fn card_face_normal(face: u32) -> vec3<f32> {
    switch (face) {
        case 0u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 1u: { return vec3<f32>(-1.0, 0.0, 0.0); }
        case 2u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 3u: { return vec3<f32>(0.0, -1.0, 0.0); }
        case 4u: { return vec3<f32>(0.0, 0.0, 1.0); }
        default: { return vec3<f32>(0.0, 0.0, -1.0); }
    }
}

// Mirror of project_to_card: (u, v, depth) of world point `p` on the card of `face`.
fn card_project(face: u32, lo: vec3<f32>, hi: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let ax = card_axes(face);
    let ext = max(hi - lo, vec3<f32>(1.0e-6));
    let uv = vec2<f32>(
        (p[ax.x] - lo[ax.x]) / ext[ax.x],
        (p[ax.y] - lo[ax.y]) / ext[ax.y],
    );
    var depth: f32;
    if (face % 2u == 0u) {
        depth = (hi[ax.z] - p[ax.z]) / ext[ax.z];
    } else {
        depth = (p[ax.z] - lo[ax.z]) / ext[ax.z];
    }
    return vec3<f32>(uv, depth);
}

// Mirror of unproject_from_card: the world point at card coordinates (uv, depth).
fn card_unproject(face: u32, lo: vec3<f32>, hi: vec3<f32>, uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ax = card_axes(face);
    let ext = max(hi - lo, vec3<f32>(1.0e-6));
    var p = vec3<f32>(0.0);
    p[ax.x] = lo[ax.x] + uv.x * ext[ax.x];
    p[ax.y] = lo[ax.y] + uv.y * ext[ax.y];
    if (face % 2u == 0u) {
        p[ax.z] = hi[ax.z] - depth * ext[ax.z];
    } else {
        p[ax.z] = lo[ax.z] + depth * ext[ax.z];
    }
    return p;
}

// Mirror of face_weights: max(n·normal, 0)² normalised over the six faces (zero -> equal).
fn card_face_weights(n: vec3<f32>) -> array<f32, 6> {
    var w: array<f32, 6>;
    var sum = 0.0;
    for (var f = 0u; f < 6u; f = f + 1u) {
        let d = max(dot(n, card_face_normal(f)), 0.0);
        w[f] = d * d;
        sum = sum + w[f];
    }
    if (sum <= 0.0) {
        for (var f = 0u; f < 6u; f = f + 1u) {
            w[f] = 1.0 / 6.0;
        }
        return w;
    }
    for (var f = 0u; f < 6u; f = f + 1u) {
        w[f] = w[f] / sum;
    }
    return w;
}

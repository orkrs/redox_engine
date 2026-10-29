// Local reflection probes (RFL-01): box influence, parallax-corrected cube lookup, blending.
// Mirrors `redox_render::reflection::probe` (box weight, parallax direction, selection).
// Bindings `t_probes` (group 0, binding 8) and `probes` (group 0, binding 9) are declared in pbr.wgsl.

struct ProbeGpu {
    // xyz: capture centre, w: blend distance.
    center_blend: vec4<f32>,
    // xyz: half extent of the influence box, w: priority.
    half_extent_prio: vec4<f32>,
}

// Matches `ProbesGpu` in Rust: scalar padding keeps `probes` at offset 16 (a vec3<u32> would
// force 16-byte alignment and move the array).
struct ProbesGpu {
    count: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    probes: array<ProbeGpu, 4>,
}

// Weight in [0, 1]: 0 outside the box, 1 deeper than the blend distance from every face.
fn probe_box_weight(p: ProbeGpu, pos: vec3<f32>) -> f32 {
    let local = pos - p.center_blend.xyz;
    let face_distance = min(min(
        p.half_extent_prio.x - abs(local.x),
        p.half_extent_prio.y - abs(local.y)),
        p.half_extent_prio.z - abs(local.z));
    if (face_distance < 0.0) {
        return 0.0;
    }
    let blend = p.center_blend.w;
    if (blend <= 0.0) {
        return 1.0;
    }
    return clamp(face_distance / blend, 0.0, 1.0);
}

// Direction from the capture centre to where the ray leaves the influence box.
// Returns `dir` unchanged when `pos` is outside the box.
fn probe_parallax_dir(p: ProbeGpu, pos: vec3<f32>, dir: vec3<f32>) -> vec3<f32> {
    let local = pos - p.center_blend.xyz;
    let half_size = p.half_extent_prio.xyz;
    if (any(abs(local) > half_size)) {
        return dir;
    }
    var exit_t = 1.0e30;
    if (abs(dir.x) > 1.0e-6) {
        let face = select(-half_size.x, half_size.x, dir.x > 0.0);
        exit_t = min(exit_t, (face - local.x) / dir.x);
    }
    if (abs(dir.y) > 1.0e-6) {
        let face = select(-half_size.y, half_size.y, dir.y > 0.0);
        exit_t = min(exit_t, (face - local.y) / dir.y);
    }
    if (abs(dir.z) > 1.0e-6) {
        let face = select(-half_size.z, half_size.z, dir.z > 0.0);
        exit_t = min(exit_t, (face - local.z) / dir.z);
    }
    if (exit_t >= 1.0e30) {
        return dir;
    }
    return local + dir * exit_t;
}

// Higher priority wins; equal priority falls back to the larger weight.
fn probe_ranks_above(pa: i32, wa: f32, pb: i32, wb: f32) -> bool {
    return pa > pb || (pa == pb && wa > wb);
}

// Prefiltered reflection from the best two probes at `pos`, blended over the global map.
// Returns the probe colour in rgb and the total probe weight in a; the caller mixes it as
// mix(global_color, rgb, a). With no probes (or none covering `pos`) a is 0 and the global map is used.
fn probe_specular(pos: vec3<f32>, r: vec3<f32>, roughness: f32) -> vec4<f32> {
    var i0 = -1;
    var w0 = 0.0;
    var p0 = 0;
    var i1 = -1;
    var w1 = 0.0;
    var p1 = 0;
    for (var i = 0u; i < probes.count; i = i + 1u) {
        let p = probes.probes[i];
        let w = probe_box_weight(p, pos);
        if (w <= 0.0) {
            continue;
        }
        let pr = i32(p.half_extent_prio.w);
        if (i0 < 0 || probe_ranks_above(pr, w, p0, w0)) {
            i1 = i0;
            w1 = w0;
            p1 = p0;
            i0 = i32(i);
            w0 = w;
            p0 = pr;
        } else if (i1 < 0 || probe_ranks_above(pr, w, p1, w1)) {
            i1 = i32(i);
            w1 = w;
            p1 = pr;
        }
    }
    var total = w0 + w1;
    if (total <= 0.0) {
        return vec4<f32>(0.0);
    }
    var n0 = w0;
    var n1 = w1;
    if (total > 1.0) {
        n0 = w0 / total;
        n1 = w1 / total;
        total = 1.0;
    }
    var colour = vec3<f32>(0.0);
    if (i0 >= 0) {
        let d0 = probe_parallax_dir(probes.probes[i0], pos, r);
        colour = colour + n0 * textureSampleLevel(t_probes, s_ibl, d0, u32(i0), roughness * 4.0).rgb;
    }
    if (i1 >= 0) {
        let d1 = probe_parallax_dir(probes.probes[i1], pos, r);
        colour = colour + n1 * textureSampleLevel(t_probes, s_ibl, d1, u32(i1), roughness * 4.0).rgb;
    }
    // colour is the weighted sum; divide by the total so that mix(global_color, rgb, total) reproduces it.
    return vec4<f32>(colour / max(total, 1.0e-6), total);
}

// The global prefiltered sample `global_color`, replaced by the probe reflection where probes cover `pos`.
fn reflection_prefiltered(pos: vec3<f32>, r: vec3<f32>, roughness: f32, global_color: vec3<f32>) -> vec3<f32> {
    if (probes.count == 0u) {
        return global_color;
    }
    let s = probe_specular(pos, r, roughness);
    return mix(global_color, s.rgb, s.a);
}

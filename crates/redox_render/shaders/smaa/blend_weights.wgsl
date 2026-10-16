// SMAA Blend Weight Calculation (Pass 2) — PRESET_HIGH
// Port of iryoku SMAA.hlsl SMAABlendingWeightCalculationPS
//
// SMAA_MAX_SEARCH_STEPS      = 16
// SMAA_MAX_SEARCH_STEPS_DIAG = 8
// SMAA_CORNER_ROUNDING       = 25
//
// Input:  edges texture (Rg8Unorm)
// Output: blend weights (Rgba8Unorm)

struct SmaaMetrics {
    rt_metrics: vec4<f32>, // (1/w, 1/h, w, h)
};

@group(0) @binding(0) var t_edges   : texture_2d<f32>;
@group(0) @binding(1) var t_area    : texture_2d<f32>;
@group(0) @binding(2) var t_search  : texture_2d<f32>;
@group(0) @binding(3) var s_linear  : sampler;
@group(0) @binding(4) var s_point   : sampler;
@group(0) @binding(5) var<uniform> u : SmaaMetrics;

const SMAA_MAX_SEARCH_STEPS: i32 = 16;
const SMAA_MAX_SEARCH_STEPS_DIAG: i32 = 8;
const SMAA_CORNER_ROUNDING_NORM: f32 = 0.25; // 25 / 100
const SMAA_AREATEX_MAX_DISTANCE: f32 = 16.0;
const SMAA_AREATEX_MAX_DISTANCE_DIAG: f32 = 20.0;
const SMAA_AREATEX_PIXEL_SIZE: vec2<f32> = vec2<f32>(1.0 / 160.0, 1.0 / 560.0);
const SMAA_AREATEX_SUBTEX_SIZE: f32 = 1.0 / 7.0;
const SMAA_SEARCHTEX_SIZE: vec2<f32> = vec2<f32>(66.0, 33.0);
const SMAA_SEARCHTEX_PACKED_SIZE: vec2<f32> = vec2<f32>(64.0, 16.0);

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) pixcoord: vec2<f32>,
    @location(2) offset0: vec4<f32>,
    @location(3) offset1: vec4<f32>,
    @location(4) offset2: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    let uv = vec2<f32>(x * 0.5 + 0.5, 1.0 - (y * 0.5 + 0.5));
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = uv;

    let m = u.rt_metrics;
    out.pixcoord = uv * m.zw;

    out.offset0 = m.xyxy * vec4<f32>(-0.25, -0.125, 1.25, -0.125) + uv.xyxy;
    out.offset1 = m.xyxy * vec4<f32>(-0.125, -0.25, -0.125, 1.25) + uv.xyxy;
    out.offset2 = m.xxyy * vec4<f32>(-2.0, 2.0, -2.0, 2.0) * f32(SMAA_MAX_SEARCH_STEPS)
                  + vec4<f32>(out.offset0.x, out.offset0.z, out.offset1.y, out.offset1.w);
    return out;
}

// ─── Helper: sample edges using linear filtering ─────────────────────────────
fn sample_edges(coord: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(t_edges, s_linear, coord, 0.0).rg;
}

fn sample_edges_point(coord: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(t_edges, s_point, coord, 0.0).rg;
}

fn sample_edges_offset(coord: vec2<f32>, offset: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(t_edges, s_linear, coord + offset * u.rt_metrics.xy, 0.0).rg;
}

// ─── SearchTex lookup ────────────────────────────────────────────────────────
fn search_length(e: vec2<f32>, offset: f32) -> f32 {
    var scale = SMAA_SEARCHTEX_SIZE * vec2<f32>(0.5, -1.0);
    var bias  = SMAA_SEARCHTEX_SIZE * vec2<f32>(offset, 1.0);
    scale += vec2<f32>(-1.0, 1.0);
    bias  += vec2<f32>( 0.5, -0.5);
    scale *= 1.0 / SMAA_SEARCHTEX_PACKED_SIZE;
    bias  *= 1.0 / SMAA_SEARCHTEX_PACKED_SIZE;
    return textureSampleLevel(t_search, s_point, scale * e + bias, 0.0).r;
}

// ─── Horizontal/Vertical search ─────────────────────────────────────────────
fn search_x_left(texcoord_in: vec2<f32>, end: f32) -> f32 {
    var tc = texcoord_in;
    var e = vec2<f32>(0.0, 1.0);
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        if (tc.x <= end || e.g <= 0.8281 || e.r != 0.0) { break; }
        e = sample_edges(tc).rg;
        tc -= vec2<f32>(2.0, 0.0) * u.rt_metrics.xy;
    }
    let offset = -(255.0 / 127.0) * search_length(e, 0.0) + 3.25;
    return u.rt_metrics.x * offset + tc.x;
}

fn search_x_right(texcoord_in: vec2<f32>, end: f32) -> f32 {
    var tc = texcoord_in;
    var e = vec2<f32>(0.0, 1.0);
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        if (tc.x >= end || e.g <= 0.8281 || e.r != 0.0) { break; }
        e = sample_edges(tc).rg;
        tc += vec2<f32>(2.0, 0.0) * u.rt_metrics.xy;
    }
    let offset = -(255.0 / 127.0) * search_length(e, 0.5) + 3.25;
    return -u.rt_metrics.x * offset + tc.x;
}

fn search_y_up(texcoord_in: vec2<f32>, end: f32) -> f32 {
    var tc = texcoord_in;
    var e = vec2<f32>(1.0, 0.0);
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        if (tc.y <= end || e.r <= 0.8281 || e.g != 0.0) { break; }
        e = sample_edges(tc).rg;
        tc -= vec2<f32>(0.0, 2.0) * u.rt_metrics.xy;
    }
    let offset = -(255.0 / 127.0) * search_length(e.gr, 0.0) + 3.25;
    return u.rt_metrics.y * offset + tc.y;
}

fn search_y_down(texcoord_in: vec2<f32>, end: f32) -> f32 {
    var tc = texcoord_in;
    var e = vec2<f32>(1.0, 0.0);
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        if (tc.y >= end || e.r <= 0.8281 || e.g != 0.0) { break; }
        e = sample_edges(tc).rg;
        tc += vec2<f32>(0.0, 2.0) * u.rt_metrics.xy;
    }
    let offset = -(255.0 / 127.0) * search_length(e.gr, 0.5) + 3.25;
    return -u.rt_metrics.y * offset + tc.y;
}

// ─── Area texture lookup ─────────────────────────────────────────────────────
fn smaa_area(dist: vec2<f32>, e1: f32, e2: f32, offset: f32) -> vec2<f32> {
    var texcoord = SMAA_AREATEX_MAX_DISTANCE * round(4.0 * vec2<f32>(e1, e2)) + dist;
    texcoord = SMAA_AREATEX_PIXEL_SIZE * texcoord + 0.5 * SMAA_AREATEX_PIXEL_SIZE;
    texcoord.y = SMAA_AREATEX_SUBTEX_SIZE * offset + texcoord.y;
    return textureSampleLevel(t_area, s_linear, texcoord, 0.0).rg;
}

fn smaa_area_diag(dist: vec2<f32>, e: vec2<f32>, offset: f32) -> vec2<f32> {
    var texcoord = SMAA_AREATEX_MAX_DISTANCE_DIAG * e + dist;
    texcoord = SMAA_AREATEX_PIXEL_SIZE * texcoord + 0.5 * SMAA_AREATEX_PIXEL_SIZE;
    texcoord.x += 0.5;
    texcoord.y += SMAA_AREATEX_SUBTEX_SIZE * offset;
    return textureSampleLevel(t_area, s_linear, texcoord, 0.0).rg;
}

// ─── Diagonal search ─────────────────────────────────────────────────────────
fn decode_diag_bilinear_access(e_in: vec2<f32>) -> vec2<f32> {
    var e = e_in;
    e.x = e.x * abs(5.0 * e.x - 5.0 * 0.75);
    return round(e);
}

fn search_diag1(texcoord_in: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    // returns (d, end_e) packed as (z=steps, w=activity, then xy for end_e)
    var coord = vec4<f32>(texcoord_in, -1.0, 1.0);
    let t = vec3<f32>(u.rt_metrics.xy, 1.0);
    var e = vec2<f32>(0.0);
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS_DIAG - 1; i++) {
        if (coord.z >= f32(SMAA_MAX_SEARCH_STEPS_DIAG - 1) || coord.w <= 0.9) { break; }
        coord = vec4<f32>(coord.xyz + t * vec3<f32>(dir, 1.0), coord.w);
        e = sample_edges(coord.xy).rg;
        coord.w = dot(e, vec2<f32>(0.5, 0.5));
    }
    return vec4<f32>(e, coord.zw);
}

fn search_diag2(texcoord_in: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    var coord = vec4<f32>(texcoord_in.x + 0.25 * u.rt_metrics.x, texcoord_in.y, -1.0, 1.0);
    let t = vec3<f32>(u.rt_metrics.xy, 1.0);
    var e = vec2<f32>(0.0);
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS_DIAG - 1; i++) {
        if (coord.z >= f32(SMAA_MAX_SEARCH_STEPS_DIAG - 1) || coord.w <= 0.9) { break; }
        coord = vec4<f32>(coord.xyz + t * vec3<f32>(dir, 1.0), coord.w);
        e = sample_edges(coord.xy).rg;
        e = decode_diag_bilinear_access(e);
        coord.w = dot(e, vec2<f32>(0.5, 0.5));
    }
    return vec4<f32>(e, coord.zw);
}

fn calculate_diag_weights(texcoord: vec2<f32>, e_in: vec2<f32>) -> vec2<f32> {
    var weights = vec2<f32>(0.0);
    var d: vec4<f32>;

    if (e_in.r > 0.0) {
        let r1 = search_diag1(texcoord, vec2<f32>(-1.0, 1.0));
        d.x = r1.z;
        d.z = r1.w;
        let end_e = r1.xy;
        d.x += select(0.0, 1.0, end_e.y > 0.9);
    } else {
        d.x = 0.0;
        d.z = 0.0;
    }
    let r2 = search_diag1(texcoord, vec2<f32>(1.0, -1.0));
    d.y = r2.z;
    d.w = r2.w;

    if (d.x + d.y > 2.0) {
        let coords = vec4<f32>(-d.x + 0.25, d.x, d.y, -d.y - 0.25) * u.rt_metrics.xyxy + texcoord.xyxy;
        var c: vec4<f32>;
        c.x = sample_edges_offset(coords.xy, vec2<f32>(-1.0, 0.0)).r;
        c.y = sample_edges_offset(coords.xy, vec2<f32>(-1.0, 0.0)).g;
        c.z = sample_edges_offset(coords.zw, vec2<f32>(1.0, 0.0)).r;
        c.w = sample_edges_offset(coords.zw, vec2<f32>(1.0, 0.0)).g;
        var c2 = vec4<f32>(c.y, c.x, c.w, c.z);
        c2 = vec4<f32>(decode_diag_bilinear_access(c2.xy), decode_diag_bilinear_access(c2.zw));
        let cc = 2.0 * vec2<f32>(c2.x, c2.z) + vec2<f32>(c2.y, c2.w);
        var cc2 = cc;
        if (d.z < 0.9) { cc2.x = 0.0; }
        if (d.w < 0.9) { cc2.y = 0.0; }
        weights += smaa_area_diag(d.xy, cc2, 0.0);
    }

    let r3 = search_diag2(texcoord, vec2<f32>(-1.0, -1.0));
    d.x = r3.z;
    d.z = r3.w;
    if (sample_edges_offset(texcoord, vec2<f32>(1.0, 0.0)).r > 0.0) {
        let r4 = search_diag2(texcoord, vec2<f32>(1.0, 1.0));
        d.y = r4.z;
        d.w = r4.w;
        let end_e2 = r4.xy;
        d.y += select(0.0, 1.0, end_e2.y > 0.9);
    } else {
        d.y = 0.0;
        d.w = 0.0;
    }

    if (d.x + d.y > 2.0) {
        let coords = vec4<f32>(-d.x, -d.x, d.y, d.y) * u.rt_metrics.xyxy + texcoord.xyxy;
        var c: vec4<f32>;
        c.x = sample_edges_offset(coords.xy, vec2<f32>(-1.0, 0.0)).g;
        c.y = sample_edges_offset(coords.xy, vec2<f32>(0.0, -1.0)).r;
        c.z = sample_edges_offset(coords.zw, vec2<f32>(1.0, 0.0)).g;
        c.w = sample_edges_offset(coords.zw, vec2<f32>(1.0, 0.0)).r;
        let cc = 2.0 * vec2<f32>(c.x, c.z) + vec2<f32>(c.y, c.w);
        var cc2 = cc;
        if (d.z < 0.9) { cc2.x = 0.0; }
        if (d.w < 0.9) { cc2.y = 0.0; }
        weights += smaa_area_diag(d.xy, cc2, 0.0).gr;
    }

    return weights;
}

// ─── Corner detection ────────────────────────────────────────────────────────
fn detect_horizontal_corner(weights_in: vec2<f32>, texcoord: vec4<f32>, d: vec2<f32>) -> vec2<f32> {
    let left_right = step(d.xy, d.yx);
    var rounding = (1.0 - SMAA_CORNER_ROUNDING_NORM) * left_right;
    rounding /= left_right.x + left_right.y;

    var factor = vec2<f32>(1.0);
    factor.x -= rounding.x * sample_edges_offset(texcoord.xy, vec2<f32>(0.0, 1.0)).r;
    factor.x -= rounding.y * sample_edges_offset(texcoord.zw, vec2<f32>(1.0, 1.0)).r;
    factor.y -= rounding.x * sample_edges_offset(texcoord.xy, vec2<f32>(0.0, -2.0)).r;
    factor.y -= rounding.y * sample_edges_offset(texcoord.zw, vec2<f32>(1.0, -2.0)).r;

    return weights_in * saturate(factor);
}

fn detect_vertical_corner(weights_in: vec2<f32>, texcoord: vec4<f32>, d: vec2<f32>) -> vec2<f32> {
    let left_right = step(d.xy, d.yx);
    var rounding = (1.0 - SMAA_CORNER_ROUNDING_NORM) * left_right;
    rounding /= left_right.x + left_right.y;

    var factor = vec2<f32>(1.0);
    factor.x -= rounding.x * sample_edges_offset(texcoord.xy, vec2<f32>(1.0, 0.0)).g;
    factor.x -= rounding.y * sample_edges_offset(texcoord.zw, vec2<f32>(1.0, 1.0)).g;
    factor.y -= rounding.x * sample_edges_offset(texcoord.xy, vec2<f32>(-2.0, 0.0)).g;
    factor.y -= rounding.y * sample_edges_offset(texcoord.zw, vec2<f32>(-2.0, 1.0)).g;

    return weights_in * saturate(factor);
}

fn saturate(v: vec2<f32>) -> vec2<f32> {
    return clamp(v, vec2<f32>(0.0), vec2<f32>(1.0));
}

// ─── Fragment shader ─────────────────────────────────────────────────────────
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var weights = vec4<f32>(0.0);
    let e = sample_edges_point(in.uv);

    if (e.g > 0.0) {
        // Diagonal detection
        var diag_weights = calculate_diag_weights(in.uv, e);

        if (diag_weights.r <= 0.0 && diag_weights.g <= 0.0) {
            // Horizontal search
            var d: vec2<f32>;
            var coords: vec3<f32>;
            coords.x = search_x_left(in.offset0.xy, in.offset2.x);
            coords.y = in.offset1.y;
            d.x = coords.x;

            let e1 = sample_edges(coords.xy).r;

            coords.z = search_x_right(in.offset0.zw, in.offset2.y);
            d.y = coords.z;

            d = abs(round(u.rt_metrics.zz * d - in.pixcoord.xx));
            let sqrt_d = sqrt(d);

            let e2 = sample_edges_offset(vec2<f32>(coords.z, coords.y), vec2<f32>(1.0, 0.0)).r;

            weights.x = smaa_area(sqrt_d, e1, e2, 0.0).x;
            weights.y = smaa_area(sqrt_d, e1, e2, 0.0).y;

            let corner_tc = vec4<f32>(coords.x, in.uv.y, coords.z, in.uv.y);
            let w = detect_horizontal_corner(weights.xy, corner_tc, d);
            weights.x = w.x;
            weights.y = w.y;
        } else {
            weights.x = diag_weights.x;
            weights.y = diag_weights.y;
        }
    }

    if (e.r > 0.0) {
        // Vertical search
        var d: vec2<f32>;
        var coords: vec3<f32>;
        coords.y = search_y_up(in.offset1.xy, in.offset2.z);
        coords.x = in.offset0.x;
        d.x = coords.y;

        let e1 = sample_edges(coords.xy).g;

        coords.z = search_y_down(in.offset1.zw, in.offset2.w);
        d.y = coords.z;

        d = abs(round(u.rt_metrics.ww * d - in.pixcoord.yy));
        let sqrt_d = sqrt(d);

        let e2 = sample_edges_offset(vec2<f32>(coords.x, coords.z), vec2<f32>(0.0, 1.0)).g;

        weights.z = smaa_area(sqrt_d, e1, e2, 0.0).x;
        weights.w = smaa_area(sqrt_d, e1, e2, 0.0).y;

        let corner_tc = vec4<f32>(in.uv.x, coords.y, in.uv.x, coords.z);
        let w = detect_vertical_corner(weights.zw, corner_tc, d);
        weights.z = w.x;
        weights.w = w.y;
    }

    return weights;
}

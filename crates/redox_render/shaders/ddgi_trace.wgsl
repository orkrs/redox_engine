// Compute DDGI probe fill: 8 octahedral rays per probe vs scene AABBs.

struct TraceParams {
    sun_dir: vec4<f32>,
    sun_rad: vec4<f32>,
    origin: vec4<f32>,
    spacing: f32,
    dims_x: u32,
    dims_y: u32,
    dims_z: u32,
    box_count: u32,
    enabled: u32,
    _pad0: u32,
    _pad1: u32,
};

struct SceneBox {
    bmin: vec4<f32>,
    bmax: vec4<f32>,
};

struct DdgiProbeGpu {
    samples: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> params: TraceParams;
@group(0) @binding(1) var<storage, read> boxes: array<SceneBox>;
@group(0) @binding(2) var<storage, read_write> probes: array<DdgiProbeGpu>;

fn sign_nz(x: f32) -> f32 {
    return select(-1.0, 1.0, x >= 0.0);
}

fn oct_decode(index: u32) -> vec3<f32> {
    let x = f32(index % 2u);
    let y = f32(index / 2u);
    let u = (x + 0.5) / 2.0;
    let v = (y + 0.5) / 4.0;
    var f = vec3<f32>(u * 2.0 - 1.0, 0.0, v * 2.0 - 1.0);
    f.y = 1.0 - abs(f.x) - abs(f.z);
    if (f.y < 0.0) {
        let ox = f.x;
        f.x = (1.0 - abs(f.z)) * sign_nz(ox);
        f.z = (1.0 - abs(ox)) * sign_nz(f.z);
    }
    return normalize(f);
}

fn ray_aabb(orig: vec3<f32>, dir: vec3<f32>, bmin: vec3<f32>, bmax: vec3<f32>) -> bool {
    let inv = 1.0 / dir;
    let t0 = (bmin - orig) * inv;
    let t1 = (bmax - orig) * inv;
    let tmin = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let tmax = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    return tmax >= max(tmin, 0.02) && tmin < 48.0;
}

fn ray_blocked(orig: vec3<f32>, dir: vec3<f32>) -> bool {
    let n = min(params.box_count, arrayLength(&boxes));
    for (var i = 0u; i < n; i++) {
        let b = boxes[i];
        if (ray_aabb(orig, dir, b.bmin.xyz, b.bmax.xyz)) {
            return true;
        }
    }
    return false;
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let probe_count = params.dims_x * params.dims_y * params.dims_z;
    let idx = gid.x;
    if (idx >= probe_count || params.enabled == 0u) {
        return;
    }
    let dx = params.dims_x;
    let dy = params.dims_y;
    let x = idx % dx;
    let y = (idx / dx) % dy;
    let z = idx / (dx * dy);
    let pos = params.origin.xyz + vec3<f32>(f32(x), f32(y), f32(z)) * params.spacing;

    let sun = normalize(params.sun_dir.xyz);
    let sun_rad = params.sun_rad.xyz;
    let sky = vec3<f32>(0.22, 0.28, 0.38);
    let ground = vec3<f32>(0.10, 0.09, 0.07);
    let height_term = abs(sin(pos.y * 0.12));

    var out_probe: DdgiProbeGpu;
    var validity = 1.0;
    for (var s = 0u; s < 8u; s++) {
        let dir = oct_decode(s);
        let blocked = ray_blocked(pos + dir * 0.05, dir);
        let hemi = dir.y * 0.5 + 0.5;
        var ambient = mix(sky, ground, 1.0 - hemi) * (0.55 + height_term * 0.15);
        var direct = sun_rad * max(dot(dir, sun), 0.0);
        if (blocked) {
            direct = vec3<f32>(0.0);
            ambient = ambient * 0.35;
            validity = 0.65;
        }
        let irr = ambient + direct;
        let w = select(0.0, validity, s == 0u);
        out_probe.samples[s] = vec4<f32>(irr, w);
    }
    probes[idx] = out_probe;
}

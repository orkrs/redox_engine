// Instanced billboard quads — vertex_index 0..3, instance_index → particle.

struct GpuParticle {
    position: vec4<f32>,
    velocity: vec4<f32>,
    color: vec4<f32>,
    life: f32,
    size: f32,
    _pad: vec2<f32>,
}

struct Camera {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> particles: array<GpuParticle>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) write_reactive: f32,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    @builtin(instance_index) iid: u32,
) -> VsOut {
    var out: VsOut;
    let p = particles[iid];
    let center = p.position.xyz;

    // Camera-facing basis from view matrix (column-major).
    let right = normalize(vec3<f32>(camera.view[0].x, camera.view[1].x, camera.view[2].x));
    let up = normalize(vec3<f32>(camera.view[0].y, camera.view[1].y, camera.view[2].y));

    var corner = vec2<f32>(-1.0, -1.0);
    if (vid == 1u) {
        corner = vec2<f32>(1.0, -1.0);
    } else if (vid == 2u) {
        corner = vec2<f32>(-1.0, 1.0);
    } else if (vid == 3u) {
        corner = vec2<f32>(1.0, 1.0);
    }
    let half = max(p.size, 1e-4);
    let world = center + right * (corner.x * half) + up * (corner.y * half);

    out.clip = camera.view_proj * vec4<f32>(world, 1.0);
    let life_a = clamp(p.life, 0.0, 1.0);
    out.color = vec4<f32>(p.color.rgb, p.color.a * life_a);
    out.write_reactive = 1.0;
    return out;
}

struct FsOut {
    @location(0) hdr: vec4<f32>,
    @location(1) reactive: f32,
}

@fragment
fn fs_main(in: VsOut) -> FsOut {
    var out: FsOut;
    out.hdr = in.color;
    out.reactive = in.write_reactive;
    return out;
}

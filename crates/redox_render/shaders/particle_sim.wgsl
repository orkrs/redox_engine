// GPU particle integration + compaction (ping-pong storage).
// MVP: CPU may upload state first; this pass can re-sim on GPU when enabled.

struct GpuParticle {
    position: vec4<f32>,
    velocity: vec4<f32>,
    color: vec4<f32>,
    life: f32,
    size: f32,
    _pad: vec2<f32>,
}

struct SimParams {
    // Matches `ParticleSimUniforms`: dt at 0, gravity packed immediately after.
    delta_time: f32,
    gravity_x: f32,
    gravity_y: f32,
    gravity_z: f32,
    capacity: u32,
    alive_count: u32,
    _pad: vec2<u32>,
}

struct DrawIndirect {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
}

@group(0) @binding(0) var<uniform> params: SimParams;
@group(0) @binding(1) var<storage, read> particles_in: array<GpuParticle>;
@group(0) @binding(2) var<storage, read_write> particles_out: array<GpuParticle>;
@group(0) @binding(3) var<storage, read_write> alive_counter: atomic<u32>;
@group(0) @binding(4) var<storage, read_write> draw_args: DrawIndirect;

@compute @workgroup_size(256)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if (idx >= params.capacity) {
        return;
    }

    var p = particles_in[idx];
    let dt = params.delta_time;

    p.life -= dt;
    if (p.life > 0.0) {
        let g = vec3<f32>(params.gravity_x, params.gravity_y, params.gravity_z);
        let vel = p.velocity.xyz + g * dt;
        let pos = p.position.xyz + vel * dt;
        p.velocity = vec4<f32>(vel, p.velocity.w);
        p.position = vec4<f32>(pos, p.position.w);
        let slot = atomicAdd(&alive_counter, 1u);
        if (slot < params.capacity) {
            particles_out[slot] = p;
        }
    }
}

// Separate entry: reset counters before the integrate pass.
@compute @workgroup_size(1)
fn cs_reset(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x == 0u) {
        atomicStore(&alive_counter, 0u);
        atomicStore(&draw_args.instance_count, 0u);
        draw_args.vertex_count = 4u;
        draw_args.first_vertex = 0u;
        draw_args.first_instance = 0u;
    }
}

// Copy alive count into indirect args after integrate.
@compute @workgroup_size(1)
fn cs_finalize(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x == 0u) {
        let alive = atomicLoad(&alive_counter);
        atomicStore(&draw_args.instance_count, alive);
    }
}

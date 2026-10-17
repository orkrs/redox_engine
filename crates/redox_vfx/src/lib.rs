//! GPU-upload-ready particle VFX (Uplift v2 Phase 8).
//!
//! CPU simulation + tightly packed [`GpuParticle`] buffers that `redox_render`
//! can later bind as storage buffers / draw as billboards. This crate does
//! **not** depend on `wgpu` — keep renderer ownership in `redox_render`.
//!
//! # Layout for a future compute bind
//! ```text
//! storage buffer @binding(N): array<GpuParticle>  // 64 bytes / particle
//! uniform: { dt, gravity, capacity, alive_count }
//! ```
//!
//! # Example
//! ```ignore
//! use redox_math::Vec3;
//! use redox_vfx::{ParticleEmitter, ParticleSystem};
//!
//! let mut system = ParticleSystem::with_capacity(100_000);
//! let mut dash = ParticleEmitter {
//!     max_particles: 512,
//!     spawn_rate: 400.0,
//!     lifetime: 0.35,
//!     initial_speed: 2.0,
//!     ..Default::default()
//! };
//! system.spawn_from_emitter(&mut dash, 1.0 / 60.0, Vec3::ZERO, Vec3::Y);
//! system.tick(1.0 / 60.0, Vec3::new(0.0, -9.8, 0.0));
//! let upload = system.buffer().upload_bytes(); // → queue.write_buffer(...)
//! ```

#![warn(missing_docs)]

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

/// Per-particle GPU record (64 bytes). Upload as a storage buffer as-is.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuParticle {
    /// World position in `xyz`; `w` is 1.
    pub position: [f32; 4],
    /// Velocity in `xyz`; `w` is unused.
    pub velocity: [f32; 4],
    /// Linear RGBA colour.
    pub color: [f32; 4],
    /// Remaining lifetime in seconds; the particle is dead at 0 or below.
    pub life: f32,
    /// Billboard half-size in world units.
    pub size: f32,
    /// Padding that keeps the record at 64 bytes.
    pub _pad: [f32; 2],
}

impl GpuParticle {
    /// Size of one record in bytes (64).
    pub const STRIDE: usize = std::mem::size_of::<Self>();

    /// Creates a particle from spawn parameters (opaque, alive for `life` seconds).
    pub fn from_spawn(pos: Vec3, vel: Vec3, color: Vec3, life: f32, size: f32) -> Self {
        Self {
            position: [pos.x, pos.y, pos.z, 1.0],
            velocity: [vel.x, vel.y, vel.z, 0.0],
            color: [color.x, color.y, color.z, 1.0],
            life,
            size,
            _pad: [0.0; 2],
        }
    }

    /// Whether the particle still has life left.
    pub fn alive(&self) -> bool {
        self.life > 0.0
    }

    /// Position as a vector.
    pub fn position_vec3(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }

    /// Velocity as a vector.
    pub fn velocity_vec3(&self) -> Vec3 {
        Vec3::new(self.velocity[0], self.velocity[1], self.velocity[2])
    }
}

/// Contiguous particle storage ready for `queue.write_buffer`.
///
/// Alive particles occupy `0..alive_count`. Dead slots are compacted on tick.
#[derive(Clone, Debug)]
pub struct ParticleBuffer {
    particles: Vec<GpuParticle>,
    alive_count: usize,
}

impl ParticleBuffer {
    /// Creates a buffer with room for `capacity` particles, all dead.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut particles = Vec::with_capacity(capacity);
        particles.resize(capacity, GpuParticle::default());
        Self {
            particles,
            alive_count: 0,
        }
    }

    /// Maximum number of particles.
    pub fn capacity(&self) -> usize {
        self.particles.len()
    }

    /// Number of alive particles (they occupy the front of the buffer).
    pub fn alive_count(&self) -> usize {
        self.alive_count
    }

    /// The alive particles.
    pub fn as_slice(&self) -> &[GpuParticle] {
        &self.particles[..self.alive_count]
    }

    /// The alive particles, mutable.
    pub fn as_mut_slice(&mut self) -> &mut [GpuParticle] {
        &mut self.particles[..self.alive_count]
    }

    /// Full backing store (capacity-sized) — useful when binding a fixed GPU buffer.
    pub fn raw(&self) -> &[GpuParticle] {
        &self.particles
    }

    /// Bytes for the alive range only (typical dynamic upload).
    pub fn upload_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(self.as_slice())
    }

    /// Bytes for the entire capacity (static buffer size).
    pub fn upload_bytes_full(&self) -> &[u8] {
        bytemuck::cast_slice(&self.particles)
    }

    /// Tries to append a particle. Returns `false` if at capacity.
    pub fn push(&mut self, particle: GpuParticle) -> bool {
        if self.alive_count >= self.particles.len() {
            return false;
        }
        self.particles[self.alive_count] = particle;
        self.alive_count += 1;
        true
    }

    /// Integrate velocity / life and compact dead particles in-place.
    pub fn tick(&mut self, delta_time: f32, gravity: Vec3) {
        let dt = delta_time.max(0.0);
        let mut write = 0usize;
        for read in 0..self.alive_count {
            let mut p = self.particles[read];
            p.life -= dt;
            if p.life <= 0.0 {
                continue;
            }
            p.velocity[0] += gravity.x * dt;
            p.velocity[1] += gravity.y * dt;
            p.velocity[2] += gravity.z * dt;
            p.position[0] += p.velocity[0] * dt;
            p.position[1] += p.velocity[1] * dt;
            p.position[2] += p.velocity[2] * dt;
            // Fade alpha with remaining life fraction stored in color.w if size>0.
            self.particles[write] = p;
            write += 1;
        }
        self.alive_count = write;
    }

    /// Marks every particle dead.
    pub fn clear(&mut self) {
        self.alive_count = 0;
    }
}

/// Uniform block matching a future compute / draw pass (16-byte aligned).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct ParticleSimUniforms {
    /// Time step in seconds.
    pub delta_time: f32,
    /// Gravity vector.
    pub gravity: [f32; 3],
    /// Buffer capacity in particles.
    pub capacity: u32,
    /// Alive particle count.
    pub alive_count: u32,
    /// Padding for 16-byte alignment.
    pub _pad: [u32; 2],
}

impl ParticleSimUniforms {
    /// Fills the block from a buffer's current counts.
    pub fn from_buffer(buffer: &ParticleBuffer, delta_time: f32, gravity: Vec3) -> Self {
        Self {
            delta_time,
            gravity: [gravity.x, gravity.y, gravity.z],
            capacity: buffer.capacity() as u32,
            alive_count: buffer.alive_count() as u32,
            _pad: [0; 2],
        }
    }
}

/// CPU-side emitter that feeds a [`ParticleBuffer`] / [`ParticleSystem`].
#[derive(Clone, Debug)]
pub struct ParticleEmitter {
    /// Maximum number of simultaneously alive particles of this emitter.
    pub max_particles: u32,
    /// Particles spawned per second.
    pub spawn_rate: f32,
    /// Lifetime of each particle in seconds.
    pub lifetime: f32,
    /// Speed at spawn, in world units per second.
    pub initial_speed: f32,
    /// Particle size at spawn.
    pub size: f32,
    /// Particle colour at spawn.
    pub color: Vec3,
    /// Cone half-angle in radians around `direction` (0 = exact).
    pub spread: f32,
    /// Fractional particles carried over between frames.
    pub accumulator: f32,
    /// Particles this emitter currently has alive.
    pub alive: u32,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            max_particles: 4096,
            spawn_rate: 64.0,
            lifetime: 2.0,
            initial_speed: 3.0,
            size: 0.05,
            color: Vec3::ONE,
            spread: 0.35,
            accumulator: 0.0,
            alive: 0,
        }
    }
}

impl ParticleEmitter {
    /// Spawns into `out` along `direction` from `origin`. Returns count spawned.
    pub fn emit_into(
        &mut self,
        out: &mut ParticleBuffer,
        delta_time: f32,
        origin: Vec3,
        direction: Vec3,
    ) -> u32 {
        self.accumulator += self.spawn_rate * delta_time;
        let dir = {
            let len_sq = direction.length_squared();
            if len_sq < 1e-8 {
                Vec3::Y
            } else {
                direction * (1.0 / len_sq.sqrt())
            }
        };
        let mut spawned = 0u32;
        let mut salt = self.alive.wrapping_add(self.accumulator.to_bits());
        while self.accumulator >= 1.0 && self.alive < self.max_particles {
            self.accumulator -= 1.0;
            if out.alive_count() >= out.capacity() {
                break;
            }
            salt = salt.wrapping_mul(1664525).wrapping_add(1013904223);
            let u = (salt & 0xFFFF) as f32 / 65535.0;
            let v = ((salt >> 16) & 0xFFFF) as f32 / 65535.0;
            let yaw = (u - 0.5) * 2.0 * self.spread;
            let pitch = (v - 0.5) * 2.0 * self.spread;
            let (sy, cy) = yaw.sin_cos();
            let (sp, cp) = pitch.sin_cos();
            // Build a basis around `dir`.
            let right = if dir.y.abs() > 0.9 {
                Vec3::X.cross(dir).normalize_or_zero()
            } else {
                Vec3::Y.cross(dir).normalize_or_zero()
            };
            let up = dir.cross(right);
            let vel_dir = (dir * cp * cy + right * sy + up * sp).normalize_or_zero();
            let vel = vel_dir * self.initial_speed;
            if out.push(GpuParticle::from_spawn(
                origin,
                vel,
                self.color,
                self.lifetime,
                self.size,
            )) {
                self.alive += 1;
                spawned += 1;
            } else {
                break;
            }
        }
        spawned
    }

    /// Legacy helper: returns newly spawned particles (allocating). Prefer [`Self::emit_into`].
    pub fn tick(&mut self, delta_time: f32, origin: Vec3) -> Vec<GpuParticle> {
        let mut tmp = ParticleBuffer::with_capacity(self.max_particles as usize);
        // Mirror current alive budget into temp capacity limit via max_particles.
        let before = self.alive;
        self.emit_into(&mut tmp, delta_time, origin, Vec3::Y);
        // `emit_into` increments alive; for the Vec API we keep that accounting.
        let _ = before;
        tmp.as_slice().to_vec()
    }

    /// Gives back `count` particles of the alive budget (e.g. after they died).
    pub fn retire(&mut self, count: u32) {
        self.alive = self.alive.saturating_sub(count);
    }
}

/// Owns a shared [`ParticleBuffer`] and drives simulation for multiple emitters.
#[derive(Clone, Debug)]
pub struct ParticleSystem {
    buffer: ParticleBuffer,
    gravity: Vec3,
}

impl ParticleSystem {
    /// Creates a system whose shared buffer holds `capacity` particles; default gravity is -9.8 on Y.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: ParticleBuffer::with_capacity(capacity),
            gravity: Vec3::new(0.0, -9.8, 0.0),
        }
    }

    /// Sets the gravity used by `tick`.
    pub fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = gravity;
    }

    /// The shared particle buffer.
    pub fn buffer(&self) -> &ParticleBuffer {
        &self.buffer
    }

    /// The shared particle buffer, mutable.
    pub fn buffer_mut(&mut self) -> &mut ParticleBuffer {
        &mut self.buffer
    }

    /// Number of alive particles.
    pub fn alive_count(&self) -> usize {
        self.buffer.alive_count()
    }

    /// Spawns the particles `emitter` owes for `delta_time` and returns how many fit in the buffer.
    pub fn spawn_from_emitter(
        &mut self,
        emitter: &mut ParticleEmitter,
        delta_time: f32,
        origin: Vec3,
        direction: Vec3,
    ) -> u32 {
        let before = self.buffer.alive_count();
        let spawned = emitter.emit_into(&mut self.buffer, delta_time, origin, direction);
        // Retire budget when buffer rejected spawns due to global capacity.
        let actual = self.buffer.alive_count().saturating_sub(before) as u32;
        if actual < spawned {
            emitter.retire(spawned - actual);
        }
        actual
    }

    /// Advances the simulation by `delta_time` seconds with the stored gravity.
    pub fn tick(&mut self, delta_time: f32) {
        let before = self.buffer.alive_count();
        self.buffer.tick(delta_time, self.gravity);
        let _died = before.saturating_sub(self.buffer.alive_count());
    }

    /// Sets the gravity, then advances the simulation.
    pub fn tick_with_gravity(&mut self, delta_time: f32, gravity: Vec3) {
        self.gravity = gravity;
        self.tick(delta_time);
    }

    /// Builds the uniform block describing the current state.
    pub fn sim_uniforms(&self, delta_time: f32) -> ParticleSimUniforms {
        ParticleSimUniforms::from_buffer(&self.buffer, delta_time, self.gravity)
    }
}

/// Camera-facing billboard corner in clip/world helpers for a future draw pass.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct BillboardVertex {
    /// World-space corner position.
    pub position: [f32; 3],
    /// Texture coordinate of the corner.
    pub uv: [f32; 2],
    /// Colour with alpha faded by remaining life.
    pub color: [f32; 4],
}

/// Expands alive particles into 6 verts (2 tris) per particle for CPU fallback draws.
pub fn expand_billboards(
    particles: &[GpuParticle],
    camera_right: Vec3,
    camera_up: Vec3,
    out: &mut Vec<BillboardVertex>,
) {
    out.clear();
    out.reserve(particles.len() * 6);
    let corners = [
        (Vec3::new(-1.0, -1.0, 0.0), [0.0, 1.0]),
        (Vec3::new(1.0, -1.0, 0.0), [1.0, 1.0]),
        (Vec3::new(1.0, 1.0, 0.0), [1.0, 0.0]),
        (Vec3::new(-1.0, -1.0, 0.0), [0.0, 1.0]),
        (Vec3::new(1.0, 1.0, 0.0), [1.0, 0.0]),
        (Vec3::new(-1.0, 1.0, 0.0), [0.0, 0.0]),
    ];
    for p in particles {
        let origin = p.position_vec3();
        let half = p.size.max(1e-4);
        let life_alpha = p.life.clamp(0.0, 1.0);
        let color = [p.color[0], p.color[1], p.color[2], p.color[3] * life_alpha];
        for (corner, uv) in corners {
            let world = origin + camera_right * (corner.x * half) + camera_up * (corner.y * half);
            out.push(BillboardVertex {
                position: [world.x, world.y, world.z],
                uv,
                color,
            });
        }
    }
}

/// ECS component linking an entity to a GPU particle emitter.
#[derive(Clone, Debug, Default)]
pub struct ParticleEmitterComponent {
    /// Emitter configuration and state.
    pub emitter: ParticleEmitter,
    /// Offset of the emission point from the entity's position.
    pub world_offset: Vec3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_ticks_and_compacts() {
        let mut buf = ParticleBuffer::with_capacity(8);
        assert!(buf.push(GpuParticle::from_spawn(
            Vec3::ZERO,
            Vec3::Y,
            Vec3::ONE,
            0.05,
            0.1
        )));
        assert!(buf.push(GpuParticle::from_spawn(
            Vec3::X,
            Vec3::ZERO,
            Vec3::ONE,
            2.0,
            0.1
        )));
        buf.tick(0.1, Vec3::ZERO);
        assert_eq!(buf.alive_count(), 1);
        assert!(buf.as_slice()[0].alive());
    }

    #[test]
    fn system_spawns_many() {
        let mut system = ParticleSystem::with_capacity(10_000);
        let mut emitter = ParticleEmitter {
            max_particles: 10_000,
            spawn_rate: 50_000.0,
            lifetime: 1.0,
            ..Default::default()
        };
        system.spawn_from_emitter(&mut emitter, 0.2, Vec3::ZERO, Vec3::Y);
        assert!(system.alive_count() > 1000);
        let bytes = system.buffer().upload_bytes();
        assert_eq!(bytes.len(), system.alive_count() * GpuParticle::STRIDE);
    }

    #[test]
    fn billboards_six_verts() {
        let p = GpuParticle::from_spawn(Vec3::ZERO, Vec3::ZERO, Vec3::ONE, 1.0, 0.5);
        let mut verts = Vec::new();
        expand_billboards(&[p], Vec3::X, Vec3::Y, &mut verts);
        assert_eq!(verts.len(), 6);
    }
}

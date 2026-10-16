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

use bytemuck::{Pod, Zeroable};
use redox_math::Vec3;

/// Per-particle GPU record (64 bytes). Upload as a storage buffer as-is.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuParticle {
    pub position: [f32; 4],
    pub velocity: [f32; 4],
    pub color: [f32; 4],
    pub life: f32,
    pub size: f32,
    pub _pad: [f32; 2],
}

impl GpuParticle {
    pub const STRIDE: usize = std::mem::size_of::<Self>();

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

    pub fn alive(&self) -> bool {
        self.life > 0.0
    }

    pub fn position_vec3(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }

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
    pub fn with_capacity(capacity: usize) -> Self {
        let mut particles = Vec::with_capacity(capacity);
        particles.resize(capacity, GpuParticle::default());
        Self {
            particles,
            alive_count: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.particles.len()
    }

    pub fn alive_count(&self) -> usize {
        self.alive_count
    }

    pub fn as_slice(&self) -> &[GpuParticle] {
        &self.particles[..self.alive_count]
    }

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

    pub fn clear(&mut self) {
        self.alive_count = 0;
    }
}

/// Uniform block matching a future compute / draw pass (16-byte aligned).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct ParticleSimUniforms {
    pub delta_time: f32,
    pub gravity: [f32; 3],
    pub capacity: u32,
    pub alive_count: u32,
    pub _pad: [u32; 2],
}

impl ParticleSimUniforms {
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
    pub max_particles: u32,
    pub spawn_rate: f32,
    pub lifetime: f32,
    pub initial_speed: f32,
    pub size: f32,
    pub color: Vec3,
    /// Cone half-angle in radians around `direction` (0 = exact).
    pub spread: f32,
    pub accumulator: f32,
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
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: ParticleBuffer::with_capacity(capacity),
            gravity: Vec3::new(0.0, -9.8, 0.0),
        }
    }

    pub fn set_gravity(&mut self, gravity: Vec3) {
        self.gravity = gravity;
    }

    pub fn buffer(&self) -> &ParticleBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut ParticleBuffer {
        &mut self.buffer
    }

    pub fn alive_count(&self) -> usize {
        self.buffer.alive_count()
    }

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

    pub fn tick(&mut self, delta_time: f32) {
        let before = self.buffer.alive_count();
        self.buffer.tick(delta_time, self.gravity);
        let _died = before.saturating_sub(self.buffer.alive_count());
    }

    pub fn tick_with_gravity(&mut self, delta_time: f32, gravity: Vec3) {
        self.gravity = gravity;
        self.tick(delta_time);
    }

    pub fn sim_uniforms(&self, delta_time: f32) -> ParticleSimUniforms {
        ParticleSimUniforms::from_buffer(&self.buffer, delta_time, self.gravity)
    }
}

/// Camera-facing billboard corner in clip/world helpers for a future draw pass.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct BillboardVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
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
        let life_alpha = p.life.max(0.0).min(1.0);
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
    pub emitter: ParticleEmitter,
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

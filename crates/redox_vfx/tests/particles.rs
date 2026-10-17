//! Behaviour of the CPU particle simulation.

use redox_math::Vec3;
use redox_vfx::{GpuParticle, ParticleBuffer, ParticleEmitter, ParticleSystem};

fn particle(life: f32) -> GpuParticle {
    GpuParticle::from_spawn(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::ONE, life, 0.1)
}

#[test]
fn push_stops_at_capacity() {
    let mut buf = ParticleBuffer::with_capacity(2);
    assert!(buf.push(particle(1.0)));
    assert!(buf.push(particle(1.0)));
    assert!(!buf.push(particle(1.0)), "third push must be rejected");
    assert_eq!(buf.alive_count(), 2);
    assert_eq!(buf.capacity(), 2);
}

#[test]
fn tick_moves_ages_and_compacts() {
    let mut buf = ParticleBuffer::with_capacity(4);
    buf.push(particle(0.05)); // dies in the first tick
    buf.push(particle(1.0));
    buf.tick(0.1, Vec3::ZERO);
    assert_eq!(buf.alive_count(), 1, "the short-lived particle is removed");
    let p = buf.as_slice()[0];
    assert!((p.position[0] - 0.1).abs() < 1e-5, "moved by velocity * dt");
    assert!((p.life - 0.9).abs() < 1e-5);
}

#[test]
fn gravity_accelerates_velocity() {
    let mut buf = ParticleBuffer::with_capacity(1);
    buf.push(particle(5.0));
    buf.tick(1.0, Vec3::new(0.0, -10.0, 0.0));
    assert!((buf.as_slice()[0].velocity[1] + 10.0).abs() < 1e-4);
}

#[test]
fn negative_delta_time_does_not_rewind() {
    let mut buf = ParticleBuffer::with_capacity(1);
    buf.push(particle(1.0));
    buf.tick(-1.0, Vec3::ZERO);
    assert_eq!(buf.alive_count(), 1);
    assert!((buf.as_slice()[0].life - 1.0).abs() < 1e-6);
}

#[test]
fn upload_bytes_cover_only_alive_particles() {
    let mut buf = ParticleBuffer::with_capacity(8);
    buf.push(particle(1.0));
    assert_eq!(buf.upload_bytes().len(), GpuParticle::STRIDE);
    assert_eq!(buf.upload_bytes_full().len(), 8 * GpuParticle::STRIDE);
    assert_eq!(GpuParticle::STRIDE, 64);
}

#[test]
fn emitter_spawns_at_its_rate() {
    let mut buf = ParticleBuffer::with_capacity(1000);
    let mut e = ParticleEmitter {
        spawn_rate: 100.0,
        ..Default::default()
    };
    let n = e.emit_into(&mut buf, 0.5, Vec3::ZERO, Vec3::Y);
    assert_eq!(n, 50);
    assert_eq!(buf.alive_count(), 50);
}

#[test]
fn system_respects_global_capacity_and_returns_budget() {
    let mut sys = ParticleSystem::with_capacity(10);
    let mut e = ParticleEmitter {
        spawn_rate: 1000.0,
        ..Default::default()
    };
    let spawned = sys.spawn_from_emitter(&mut e, 1.0, Vec3::ZERO, Vec3::Y);
    assert_eq!(spawned, 10, "only the buffer capacity fits");
    assert_eq!(sys.alive_count(), 10);
    assert!(
        e.alive <= 10,
        "rejected spawns are given back to the emitter"
    );
}

#[test]
fn clear_kills_everything() {
    let mut sys = ParticleSystem::with_capacity(4);
    sys.buffer_mut().push(particle(1.0));
    sys.buffer_mut().clear();
    assert_eq!(sys.alive_count(), 0);
}

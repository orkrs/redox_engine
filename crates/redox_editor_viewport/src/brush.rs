use redox_ecs::Entity;
use redox_math::{Quat, Vec3};
use redox_physics::PhysicsContext;
use std::path::PathBuf;

/// Settings of the scatter brush used to paint props into the world.
#[derive(Clone, Debug)]
pub struct BrushSettings {
    /// Brush radius in world units.
    pub radius: f32,
    /// Controls how many instances one application places (about 1.5 per unit).
    pub density: f32,
    /// Minimum and maximum scale factor of a placed instance.
    pub scale_range: (f32, f32),
    /// Minimum and maximum yaw of a placed instance, in degrees.
    pub yaw_range: (f32, f32),
    /// Asset paths the brush picks from.
    pub palette: Vec<PathBuf>,
    /// Whether placed instances are projected onto the surface under the brush.
    pub surface_snap: bool,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            radius: 3.0,
            density: 8.0,
            scale_range: (0.8, 1.2),
            yaw_range: (0.0, 360.0),
            palette: Vec::new(),
            surface_snap: true,
        }
    }
}

/// One brush application: where it touched the surface.
#[derive(Clone, Debug)]
pub struct BrushStroke {
    /// Hit point on the surface.
    pub center: Vec3,
    /// Surface normal at the hit point.
    pub normal: Vec3,
}

/// Deterministic scatter positions inside a disc of `radius` around `center`; the same `seed` gives
/// the same layout.
pub fn scatter_positions(center: Vec3, radius: f32, density: f32, seed: u64) -> Vec<Vec3> {
    scatter_poses(center, radius, density, seed)
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

/// Positions + random yaw for brush paint.
pub fn scatter_poses(center: Vec3, radius: f32, density: f32, seed: u64) -> Vec<(Vec3, f32)> {
    let count = (density * 1.5) as usize;
    let mut out = Vec::with_capacity(count);
    let mut rng = seed;
    for _ in 0..count {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let angle = (rng as f32 / u64::MAX as f32) * std::f32::consts::TAU;
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let r = (rng as f32 / u64::MAX as f32).sqrt() * radius;
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let yaw = (rng as f32 / u64::MAX as f32) * std::f32::consts::TAU;
        out.push((
            Vec3::new(
                center.x + r * angle.cos(),
                center.y,
                center.z + r * angle.sin(),
            ),
            yaw,
        ));
    }
    out
}

/// Snap object resting on surface. Half-extent offsets the contact so a unit cube sits on the hit.
pub fn place_on_surface(
    pos: Vec3,
    half_height: f32,
    physics: Option<&mut PhysicsContext>,
    exclude: &[Entity],
) -> (Vec3, Quat) {
    if let Some(phys) = physics {
        let origin = pos + Vec3::Y * 80.0;
        if let Some((hit, normal, _)) = phys.cast_ray_with_normal(origin, -Vec3::Y, 200.0, exclude)
        {
            let n = normal.normalize_or_zero();
            let n = if n.length_squared() < 1e-6 {
                Vec3::Y
            } else {
                n
            };
            let rot = Quat::from_rotation_arc(Vec3::Y, n);
            return (hit + n * half_height, rot);
        }
    }
    // Floor plane y = 0
    (Vec3::new(pos.x, half_height, pos.z), Quat::IDENTITY)
}

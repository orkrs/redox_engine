//! Gizmo snapping: grid (existing), surface ray, vertex pick API.

use redox_ecs::Entity;
use redox_math::{Quat, Transform, Vec3};
use redox_physics::PhysicsContext;

use crate::brush::place_on_surface;
use crate::gizmo::GizmoState;

/// How translation snaps during gizmo drags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GizmoSnapMode {
    /// Quantize along axes using [`GizmoState::grid`].
    #[default]
    Grid,
    /// Project onto physics or floor hit under the cursor.
    Surface,
    /// Snap to nearest candidate vertex in world space.
    Vertex,
}

/// Settings combined with grid step from [`GizmoState`].
#[derive(Clone, Copy, Debug)]
pub struct GizmoSnapSettings {
    pub mode: GizmoSnapMode,
    /// Maximum distance for vertex snap.
    pub vertex_radius: f32,
}

impl Default for GizmoSnapSettings {
    fn default() -> Self {
        Self {
            mode: GizmoSnapMode::Grid,
            vertex_radius: 0.35,
        }
    }
}

/// Applies surface snap; when physics misses, position is unchanged.
pub fn surface_snap_translation(
    target: Vec3,
    half_height: f32,
    physics: Option<&mut PhysicsContext>,
    exclude: &[Entity],
) -> Vec3 {
    let (snapped, _) = place_on_surface(target, half_height, physics, exclude);
    if physics.is_some() {
        snapped
    } else {
        target
    }
}

/// Finds the closest vertex among `candidates` to `world_point` within `radius`.
pub fn snap_to_nearest_vertex(world_point: Vec3, candidates: &[Vec3], radius: f32) -> Option<Vec3> {
    let r2 = radius * radius;
    candidates
        .iter()
        .filter(|v| (*v - world_point).length_squared() <= r2)
        .min_by(|a, b| {
            let da = (*a - world_point).length_squared();
            let db = (*b - world_point).length_squared();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

/// Final translation for a gizmo drag after snap mode.
pub fn apply_snap_to_translation(
    gizmo: &GizmoState,
    snap: GizmoSnapSettings,
    raw: Vec3,
    entity: Transform,
    physics: Option<&mut PhysicsContext>,
    exclude: &[Entity],
    vertex_cloud: &[Vec3],
) -> Vec3 {
    match snap.mode {
        GizmoSnapMode::Grid => raw,
        GizmoSnapMode::Surface => {
            let base = entity.translation + raw;
            let snapped = surface_snap_translation(base, 0.5, physics, exclude);
            if physics.is_some() {
                snapped - entity.translation
            } else {
                Vec3::ZERO
            }
        }
        GizmoSnapMode::Vertex => {
            let base = entity.translation + raw;
            snap_to_nearest_vertex(base, vertex_cloud, snap.vertex_radius)
                .map(|v| v - entity.translation)
                .unwrap_or(Vec3::ZERO)
        }
    }
}

/// Alignment rotation from surface hit normal.
pub fn align_rotation_to_normal(normal: Vec3) -> Quat {
    let n = normal.normalize_or_zero();
    if n.length_squared() < 1e-6 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(Vec3::Y, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_snap_without_physics_does_not_move() {
        let start = Vec3::new(3.0, 1.0, 2.0);
        let gizmo = GizmoState::new();
        let out = apply_snap_to_translation(
            &gizmo,
            GizmoSnapSettings {
                mode: GizmoSnapMode::Surface,
                ..Default::default()
            },
            Vec3::new(1.0, 0.0, 0.0),
            Transform::from_translation(start),
            None,
            &[],
            &[],
        );
        assert!(out.length_squared() < 1e-6);
    }

    #[test]
    fn vertex_snap_picks_nearest() {
        let cloud = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0)];
        let hit = snap_to_nearest_vertex(Vec3::new(0.4, 0.0, 0.0), &cloud, 1.0);
        assert_eq!(hit, Some(Vec3::ZERO));
    }
}
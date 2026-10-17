use redox_ecs::{Entity, World};
use redox_math::{Mat4, Quat, Transform, Vec3};
use redox_reflect::Reflect;
use redox_reflect::TypeRegistry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GizmoMode {
    #[default]
    Translate,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GizmoAxis {
    #[default]
    X,
    Y,
    Z,
    Screen,
}

#[derive(Clone, Debug, Default)]
pub struct GridSettings {
    pub enabled: bool,
    pub step: f32,
    pub angle_step: f32,
}

#[derive(Clone, Debug, Default)]
pub struct GizmoState {
    pub mode: GizmoMode,
    pub axis: Option<GizmoAxis>,
    pub drag_start: Option<Vec3>,
    pub entity_start: Option<Transform>,
    pub grid: GridSettings,
}

impl GizmoState {
    pub fn new() -> Self {
        Self {
            mode: GizmoMode::Translate,
            grid: GridSettings {
                enabled: true,
                step: 0.5,
                angle_step: 15.0,
            },
            ..Default::default()
        }
    }

    pub fn start_drag(
        &mut self,
        ray_origin: Vec3,
        ray_dir: Vec3,
        entity_pos: Vec3,
        transform: Transform,
        camera_forward: Vec3,
    ) -> bool {
        // Simple axis pick: test 3 axes as cylinders (approx with ray distance)
        let axes = [
            (GizmoAxis::X, Vec3::X),
            (GizmoAxis::Y, Vec3::Y),
            (GizmoAxis::Z, Vec3::Z),
        ];
        let mut best: Option<(GizmoAxis, f32)> = None;
        for (axis, dir) in axes {
            if let Some(t) = ray_intersect_axis(ray_origin, ray_dir, entity_pos, dir) {
                let hit = ray_origin + ray_dir * t;
                let dist = (hit - entity_pos).length();
                if dist < 1.5 {
                    // within gizmo length
                    let score = (hit - ray_origin).length();
                    if best.is_none_or(|b| score < b.1) {
                        best = Some((axis, score));
                    }
                }
            }
        }
        if let Some((axis, _)) = best {
            self.axis = Some(axis);
            // For translate, find start point on plane perpendicular to camera
            let plane_normal = camera_forward;
            if let Some(pt) = ray_plane_intersect(ray_origin, ray_dir, entity_pos, plane_normal) {
                self.drag_start = Some(pt);
                self.entity_start = Some(transform);
                return true;
            }
        }
        false
    }

    pub fn update_drag(
        &self,
        ray_origin: Vec3,
        ray_dir: Vec3,
        entity_pos: Vec3,
        camera_forward: Vec3,
    ) -> Option<Vec3> {
        let axis = self.axis?;
        let start = self.drag_start?;
        let plane_normal = camera_forward;
        let cur = ray_plane_intersect(ray_origin, ray_dir, entity_pos, plane_normal)?;
        let delta = cur - start;
        let axis_dir = match axis {
            GizmoAxis::X => Vec3::X,
            GizmoAxis::Y => Vec3::Y,
            GizmoAxis::Z => Vec3::Z,
            GizmoAxis::Screen => plane_normal,
        };
        let mut offset = axis_dir * delta.dot(axis_dir);
        if self.grid.enabled {
            let step = self.grid.step.max(0.001);
            offset = (offset / step).round() * step;
        }
        Some(offset)
    }

    pub fn apply_drag(
        &self,
        world: &mut World,
        entity: Entity,
        offset: Vec3,
        _registry: &TypeRegistry,
        undo: &mut crate::undo::UndoStack,
    ) {
        if let Some(start) = self.entity_start {
            let new_transform = self.dragged_transform(start, offset);
            // Record undo
            if let Some(old_val) = world
                .get_component::<Transform>(entity)
                .map(|t| t.to_value())
            {
                let new_val = new_transform.to_value();
                undo.push(crate::undo::UndoCommand::SetComponent {
                    entity,
                    type_name: "Transform".into(),
                    before: Some(old_val),
                    after: Some(new_val),
                });
            }
            if let Some(t) = world.get_component_mut::<Transform>(entity) {
                *t = new_transform;
            }
        }
    }

    /// World direction of the dragged axis (Y when no axis or the screen plane is active).
    fn axis_direction(&self) -> Vec3 {
        match self.axis {
            Some(GizmoAxis::X) => Vec3::X,
            Some(GizmoAxis::Z) => Vec3::Z,
            Some(GizmoAxis::Y) | Some(GizmoAxis::Screen) | None => Vec3::Y,
        }
    }

    /// Transform produced by dragging `start` by `offset` (the axis-projected drag delta).
    ///
    /// Rotate: spins about the dragged axis, 1 world unit of drag = 1 radian, snapped to
    /// `grid.angle_step` degrees. Scale: signed distance along the axis, so dragging
    /// against the axis shrinks.
    pub fn dragged_transform(&self, start: Transform, offset: Vec3) -> Transform {
        let mut out = start;
        let axis_dir = self.axis_direction();
        match self.mode {
            GizmoMode::Translate => out.translation += offset,
            GizmoMode::Scale => {
                let factor = (1.0 + offset.dot(axis_dir) * 0.5).max(MIN_SCALE_FACTOR);
                out.scale *= factor;
            }
            GizmoMode::Rotate => {
                let angle = snap_angle(offset.dot(axis_dir), &self.grid);
                out.rotation = Quat::from_axis_angle(axis_dir, angle) * start.rotation;
            }
        }
        out
    }

    pub fn end_drag(&mut self) {
        self.axis = None;
        self.drag_start = None;
        self.entity_start = None;
    }
}

/// Smallest uniform scale multiplier a single drag may apply.
const MIN_SCALE_FACTOR: f32 = 0.01;

/// Snaps `angle_rad` to the grid's `angle_step` (degrees) when snapping is enabled.
fn snap_angle(angle_rad: f32, grid: &GridSettings) -> f32 {
    let step = grid.angle_step.to_radians();
    if grid.enabled && step > f32::EPSILON {
        (angle_rad / step).round() * step
    } else {
        angle_rad
    }
}

pub fn ray_intersect_axis(
    ray_origin: Vec3,
    ray_dir: Vec3,
    axis_origin: Vec3,
    axis_dir: Vec3,
) -> Option<f32> {
    // Closest distance between two lines
    let w0 = ray_origin - axis_origin;
    let a = ray_dir.dot(ray_dir);
    let b = ray_dir.dot(axis_dir);
    let c = axis_dir.dot(axis_dir);
    let d = ray_dir.dot(w0);
    let e = axis_dir.dot(w0);
    let denom = a * c - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (b * e - c * d) / denom;
    if t < 0.0 { None } else { Some(t) }
}
pub fn ray_plane_intersect(
    ray_origin: Vec3,
    ray_dir: Vec3,
    plane_point: Vec3,
    plane_normal: Vec3,
) -> Option<Vec3> {
    let denom = plane_normal.dot(ray_dir);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_point - ray_origin).dot(plane_normal) / denom;
    if t < 0.0 {
        None
    } else {
        Some(ray_origin + ray_dir * t)
    }
}
pub fn screen_project(pos: Vec3, view_proj: Mat4, viewport_size: (u32, u32)) -> Option<Vec3> {
    let clip = view_proj * pos.extend(1.0);
    if clip.w.abs() < 1e-6 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(Vec3::new(
        (ndc.x * 0.5 + 0.5) * viewport_size.0 as f32,
        (1.0 - (ndc.y * 0.5 + 0.5)) * viewport_size.1 as f32,
        ndc.z,
    ))
}

/// Generate world ray from viewport UV (0-1) and camera matrices
pub fn screen_ray(
    uv: redox_math::Vec2,
    view: Mat4,
    proj: Mat4,
    viewport_size: (u32, u32),
) -> (Vec3, Vec3) {
    // Convert UV to NDC
    let ndc_x = uv.x * 2.0 - 1.0;
    let ndc_y = 1.0 - uv.y * 2.0;
    let inv_view_proj = (proj * view).inverse();
    let near = inv_view_proj * Vec3::new(ndc_x, ndc_y, 0.0).extend(1.0);
    let far = inv_view_proj * Vec3::new(ndc_x, ndc_y, 1.0).extend(1.0);
    let near_w = near.w;
    let far_w = far.w;
    let near_pos = near.truncate() / near_w;
    let far_pos = far.truncate() / far_w;
    let origin = near_pos;
    let dir = (far_pos - near_pos).normalize_or_zero();
    let _ = viewport_size;
    (origin, dir)
}

/// Focus camera on AABB (called on F)
pub fn focus_camera_on_aabb(
    camera: &mut crate::camera::EditorCamera,
    aabb_center: Vec3,
    aabb_size: f32,
) {
    let center = aabb_center;
    let dist = (aabb_size * 0.6).max(2.0) / (camera.fov_y * 0.5).tan();
    let dir = (camera.eye - center).normalize_or_zero();
    let dir = if dir.length_squared() < 1e-6 {
        Vec3::new(0.0, 0.3, 1.0).normalize()
    } else {
        dir
    };
    camera.eye = center + dir * dist;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::UndoStack;
    use redox_ecs::World;
    use redox_math::{Transform, Vec3};
    use redox_reflect::TypeRegistry;
    #[test]
    fn gizmo_translate_with_snap() {
        let mut world = World::new();
        let mut registry = TypeRegistry::new();
        registry.register::<Transform>();
        let e = world.spawn();
        world.add_component(e, Transform::from_translation(Vec3::ZERO));
        let mut gizmo = GizmoState::new();
        gizmo.grid.enabled = true;
        gizmo.grid.step = 0.5;
        // Simulate drag from origin along X by 2.3 -> snapped to 2.5
        let start = Transform::from_translation(Vec3::ZERO);
        gizmo.entity_start = Some(start);
        gizmo.axis = Some(GizmoAxis::X);
        gizmo.drag_start = Some(Vec3::ZERO);
        gizmo.mode = GizmoMode::Translate;
        let mut undo = UndoStack::default();
        let offset = Vec3::new(2.3, 0.0, 0.0);
        // snapped offset should be 2.5
        let snapped = (offset / 0.5).round() * 0.5;
        gizmo.apply_drag(&mut world, e, snapped, &registry, &mut undo);
        let t = world.get_component::<Transform>(e).unwrap();
        assert!((t.translation.x - 2.5).abs() < 0.001);
        // Undo should revert
        assert!(undo.can_undo());
        undo.undo(&mut world, &registry);
        let t2 = world.get_component::<Transform>(e).unwrap();
        assert!(t2.translation.x.abs() < 0.001);
    }
    #[test]
    fn focus_camera_centers() {
        let mut cam = crate::camera::EditorCamera::default();
        cam.eye = Vec3::new(10.0, 10.0, 10.0);
        focus_camera_on_aabb(&mut cam, Vec3::ZERO, 2.0);
        // After focus, camera should be looking at origin, distance ~ 2.0 / tan(fov/2)
        let dir = (cam.eye - Vec3::ZERO).normalize();
        assert!(dir.length() > 0.9);
    }

    fn drag_state(mode: GizmoMode, axis: GizmoAxis, snap: bool) -> GizmoState {
        GizmoState {
            mode,
            axis: Some(axis),
            grid: GridSettings {
                enabled: snap,
                step: 0.5,
                angle_step: 15.0,
            },
            ..Default::default()
        }
    }

    #[test]
    fn rotate_spins_about_dragged_axis() {
        let g = drag_state(GizmoMode::Rotate, GizmoAxis::X, false);
        let out = g.dragged_transform(Transform::default(), Vec3::X * 0.5);
        let rotated = out.rotation * Vec3::Y;
        let expected = Quat::from_rotation_x(0.5) * Vec3::Y;
        assert!((rotated - expected).length() < 1.0e-5);
    }

    #[test]
    fn rotate_snaps_to_angle_step_in_degrees() {
        let g = drag_state(GizmoMode::Rotate, GizmoAxis::Y, true);
        // 0.27 rad = 15.47 deg snaps to 15 deg.
        let out = g.dragged_transform(Transform::default(), Vec3::Y * 0.27);
        let (_, angle) = out.rotation.to_axis_angle();
        assert!((angle.to_degrees() - 15.0).abs() < 1.0e-3);
    }

    #[test]
    fn scale_shrinks_when_dragged_against_axis() {
        let g = drag_state(GizmoMode::Scale, GizmoAxis::X, false);
        let grown = g.dragged_transform(Transform::default(), Vec3::X);
        let shrunk = g.dragged_transform(Transform::default(), -Vec3::X);
        assert!(grown.scale.x > 1.0);
        assert!(shrunk.scale.x < 1.0 && shrunk.scale.x > 0.0);
    }
}

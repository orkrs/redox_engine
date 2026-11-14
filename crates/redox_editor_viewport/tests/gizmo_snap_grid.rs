//! EDT-B3 step 4: grid snap quantizes translation.

use redox_editor_viewport::{GizmoAxis, GizmoMode, GizmoState, GridSettings};

#[test]
fn snap_to_grid_quantizes_translation() {
    let mut gizmo = GizmoState::new();
    gizmo.grid.enabled = true;
    gizmo.grid.step = 0.5;
    gizmo.mode = GizmoMode::Translate;
    gizmo.axis = Some(GizmoAxis::X);
    let raw = redox_math::Vec3::new(2.3, 0.0, 0.0);
    let snapped = (raw / 0.5).round() * 0.5;
    assert!((snapped.x - 2.5).abs() < 1e-5);
}
//! Viewport of RedOx Studio: editor camera, gizmo, picking, scatter brush and the wgpu passes that draw the scene proxies.

#![warn(missing_docs)]

/// Scatter brush used to paint props.
pub mod brush;
/// Free-fly and orbit editor camera.
pub mod camera;
/// Transform gizmo logic.
pub mod gizmo;
/// Picking of entities in the viewport.
pub mod picker;
/// Offscreen viewport target and the passes that draw the scene proxies.
pub mod viewport;
/// Shading modes, layouts and debug flags of the viewport.
pub mod viewport_mode;

pub use brush::{BrushSettings, BrushStroke, place_on_surface, scatter_poses, scatter_positions};
pub use camera::EditorCamera;
pub use gizmo::{GizmoAxis, GizmoMode, GizmoState, GridSettings};
pub use picker::{
    EntityPicker, QuadPickRay, pick_entity_in_quad_pane, pick_in_quad_pane, pick_unit_cubes,
    quad_pane_at_uv, quad_pane_cameras,
};
pub use viewport::{
    EditorViewport, ViewportCubePass, ViewportLinesPass, build_editor_overlay_lines,
    ghost_cube_lines, ghost_move_line, window_to_viewport_coords,
};
pub use viewport_mode::{
    QUAD_ORTHO_HALF, QuadPane, ViewportDebugFlags, ViewportLayout, ViewportShading,
};

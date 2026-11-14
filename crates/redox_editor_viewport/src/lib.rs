//! Viewport of RedOx Studio: editor camera, gizmo, picking, scatter brush and the wgpu passes that draw the scene proxies.

#![warn(missing_docs)]

/// glTF → GPU mesh bridge for the editor viewport.
pub mod asset_bridge;
/// Scatter brush used to paint props.
pub mod brush;
/// Free-fly and orbit editor camera.
pub mod camera;
/// Transform gizmo logic.
pub mod gizmo;
/// Picking of entities in the viewport.
pub mod picker;
/// Shading mode resolution against engine capabilities.
pub mod shading;
/// Gizmo snap modes (grid / surface / vertex).
pub mod snap;
/// Offscreen viewport target and the passes that draw the scene proxies.
pub mod viewport;
/// Shading modes, layouts and debug flags of the viewport.
pub mod viewport_mode;
/// Applies runtime toggles on the engine [`Renderer`].
pub mod viewport_runtime;
/// Viewport PNG readback.
pub mod viewport_screenshot;
/// Serializable GI / sky / fog toggles.
pub mod viewport_settings;
/// FPS / CPU / GPU overlay lines.
pub mod viewport_stats;

pub use asset_bridge::{
    ResolvedViewportMesh, ViewportAssetKind, ViewportDrawItem, ViewportMeshCache,
    ViewportSceneAssetRef, gltf_mesh_to_render_mesh,
};
pub use brush::{BrushSettings, BrushStroke, place_on_surface, scatter_poses, scatter_positions};
pub use camera::EditorCamera;
pub use gizmo::{GizmoAxis, GizmoMode, GizmoState, GridSettings};
pub use picker::{
    EntityPicker, QuadPickRay, pick_entity_in_quad_pane, pick_in_quad_pane, pick_unit_cubes,
    quad_pane_at_uv, quad_pane_cameras,
};
pub use shading::{ShadingAvailability, ViewportShadingPlan, apply_shading_plan};
pub use snap::{
    GizmoSnapMode, GizmoSnapSettings, apply_snap_to_translation, snap_to_nearest_vertex,
};
pub use viewport::{
    EditorViewport, ViewportCubePass, ViewportLinesPass, build_editor_overlay_lines,
    ghost_cube_lines, ghost_move_line, mesh_wireframe_lines, window_to_viewport_coords,
};
pub use viewport_mode::{
    QUAD_ORTHO_HALF, QuadPane, ViewportDebugFlags, ViewportLayout, ViewportShading,
};
pub use viewport_runtime::apply_viewport_runtime;
pub use viewport_screenshot::{read_rgba8_texture, screenshot_texture_to_png, write_viewport_png};
pub use viewport_settings::{ViewportRuntimeSettings, ViewportToggleStatus};
pub use viewport_stats::ViewportStatsOverlay;

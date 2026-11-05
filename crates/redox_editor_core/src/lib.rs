//! RedOx Editor Core — non-visual business logic for RedOx Studio.

#![warn(missing_docs)]

/// Scatter brush used to paint props.
pub mod brush;
/// Free-fly and orbit editor camera.
pub mod camera;
/// Version-control status of actor files.
pub mod git;
/// Transform gizmo logic.
pub mod gizmo;
pub mod picker;
/// Selection set and primary selection.
pub mod selection;
/// Undo and redo commands.
pub mod undo;
pub mod viewport;
pub mod viewport_mode;
/// The open world: partition streamer, manifest and root directory.
pub mod world_session;

pub use brush::{BrushSettings, BrushStroke, place_on_surface, scatter_poses, scatter_positions};
pub use camera::EditorCamera;
pub use git::{ActorGitStatus, WorldGitTracker};
pub use gizmo::{GizmoAxis, GizmoMode, GizmoState, GridSettings};
pub use picker::{
    EntityPicker, QuadPickRay, pick_entity_in_quad_pane, pick_in_quad_pane, pick_unit_cubes,
    quad_pane_at_uv, quad_pane_cameras,
};
pub use selection::{Selection, Visibility};
pub use undo::{UndoCommand, UndoStack};
pub use viewport::{
    EditorViewport, ViewportCubePass, ViewportLinesPass, build_editor_overlay_lines,
    egui_to_viewport_coords, ghost_cube_lines, ghost_move_line,
};
pub use viewport_mode::{
    QUAD_ORTHO_HALF, QuadPane, ViewportDebugFlags, ViewportLayout, ViewportShading,
};
pub use world_session::WorldSession;

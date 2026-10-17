//! RedOx Editor UI — egui_dock panels

#![warn(missing_docs)]

/// Asset browser panel.
pub mod asset_browser;
/// Brush tool panel.
pub mod brush_panel;
/// Command palette.
pub mod command_palette;
pub mod context;
/// Dock layout and tabs.
pub mod dock;
/// Version-control overlay.
pub mod git_overlay;
/// Component inspector.
pub mod inspector;
pub mod node_canvas;
/// World outliner.
pub mod outliner;
pub mod packager;
/// Frame profiler panel.
pub mod profiler_panel;

pub use asset_browser::{AssetKind, AssetSpawnRequest, EditorMeshProxy};
pub use brush_panel::BrushPanel;
pub use context::EditorUiCtx;
pub use dock::{EditorDock, Tab};
pub use outliner::OutlinerAction;
pub use packager::{BakeReport, BakeStatus, PackagerPanel};

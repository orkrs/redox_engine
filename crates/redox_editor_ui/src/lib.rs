//! RedOx Editor UI — egui_dock panels

#![warn(missing_docs)]

/// Asset browser panel.
pub mod asset_browser;
/// Diagnostics window for the report of opening an assets root.
pub mod asset_report;
/// Brush tool panel.
pub mod brush_panel;
/// Command palette.
pub mod command_palette;
pub mod context;
/// Dock layout and tabs.
pub mod dock;
/// Version-control overlay.
pub mod git_overlay;
/// egui adapters over the egui-free editor logic.
pub mod input_adapters;
/// Component inspector.
pub mod inspector;
pub mod node_canvas;
/// World outliner.
pub mod outliner;
pub mod packager;
/// Frame profiler panel.
pub mod profiler_panel;

pub use asset_browser::{AssetKind, AssetRequest, AssetSpawnRequest, EditorMeshProxy};
pub use asset_report::{AssetReportPanel, ReportAction, has_problems};
pub use brush_panel::BrushPanel;
pub use context::EditorUiCtx;
pub use dock::{EditorDock, Tab};
pub use input_adapters::{egui_to_viewport_coords, undo_hotkey};
pub use outliner::OutlinerAction;
pub use packager::{BakeReport, BakeStatus, PackagerPanel};

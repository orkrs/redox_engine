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
/// File menu, unsaved-changes prompt, path dialogs and load report.
pub mod file_menu;
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
/// Undo history panel.
pub mod undo_history;
/// Issues / validation panel model without egui.
pub mod validation_model;
/// egui Issues tab.
pub mod validation_panel;
/// Crash-recovery prompt and the Preferences window.
pub mod recovery_dialog;

pub use asset_browser::{AssetKind, AssetRequest, AssetSpawnRequest, EditorMeshProxy};
pub use asset_report::{AssetReportPanel, ReportAction, has_problems};
pub use brush_panel::BrushPanel;
pub use context::EditorUiCtx;
pub use dock::{EditorDock, Tab};
pub use input_adapters::{egui_to_viewport_coords, undo_hotkey};
pub use outliner::OutlinerAction;
pub use packager::{BakeReport, BakeStatus, PackagerPanel};
pub use recovery_dialog::{
    PreferencesWindow, RecoveryAnswer, RecoveryDialog, RecoveryPrompt, format_hhmm,
};
pub use validation_model::{SeverityFilter, ValidationPanelAction, ValidationPanelModel};
pub use validation_panel::ValidationPanel;

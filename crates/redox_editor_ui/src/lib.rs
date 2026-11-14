//! RedOx Editor UI — egui_dock panels

#![warn(missing_docs)]

/// Asset browser panel.
pub mod asset_browser;
pub mod audit_panel;
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
pub use git_overlay::{GitOverlay, GitOverlayAction};
/// File menu, unsaved-changes prompt, path dialogs and load report.
pub mod file_menu;
pub mod import_dialog_panel;
pub mod import_log_panel;
/// egui adapters over the egui-free editor logic.
pub mod input_adapters;
/// Component inspector.
pub mod inspector;
pub mod material_canvas;
pub mod material_editor;
pub mod material_panel;
pub mod node_canvas;
/// World outliner.
pub mod outliner;
pub mod packager;
/// Frame profiler panel.
pub mod profiler_panel;
/// Console panel with filters.
pub mod console_panel;
/// Undo history panel.
pub mod undo_history;
/// Issues / validation panel model without egui.
pub mod validation_model;
/// egui Issues tab.
pub mod validation_panel;
/// Crash-recovery prompt and the Preferences window.
pub mod recovery_dialog;
pub mod reference_panel;

pub use asset_browser::{AssetKind, AssetRequest, AssetSpawnRequest, EditorMeshProxy};
pub use audit_panel::AuditPanel;
pub use import_dialog_panel::ImportDialogPanel;
pub use import_log_panel::ImportLogPanel;
pub use reference_panel::ReferenceViewerPanel;
pub use asset_report::{AssetReportPanel, ReportAction, has_problems};
pub use brush_panel::BrushPanel;
pub use context::EditorUiCtx;
pub use dock::{EditorDock, Tab};
pub use input_adapters::{egui_to_viewport_coords, undo_hotkey};
pub use material_editor::{MaterialEditorSession, MaterialOpenRequest};
pub use outliner::OutlinerAction;
pub use packager::{BakeReport, BakeStatus, PackagerPanel};
pub use recovery_dialog::{
    PreferencesWindow, RecoveryAnswer, RecoveryDialog, RecoveryPrompt, format_hhmm,
};
pub use validation_model::{SeverityFilter, ValidationPanelAction, ValidationPanelModel};
pub use validation_panel::ValidationPanel;

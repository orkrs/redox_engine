//! RedOx Editor UI — egui_dock panels

pub mod asset_browser;
pub mod brush_panel;
pub mod command_palette;
pub mod context;
pub mod dock;
pub mod git_overlay;
pub mod inspector;
pub mod node_canvas;
pub mod outliner;
pub mod packager;
pub mod profiler_panel;

pub use asset_browser::{AssetKind, AssetSpawnRequest, EditorMeshProxy};
pub use brush_panel::BrushPanel;
pub use context::EditorUiCtx;
pub use dock::{EditorDock, Tab};
pub use outliner::OutlinerAction;
pub use packager::{BakeReport, BakeStatus, PackagerPanel};

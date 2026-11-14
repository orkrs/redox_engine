//! Material editor session: document, graph view, compile report and preview settings.

use std::path::PathBuf;

use redox_asset::AssetGuid;
use redox_editor_assets::{
    CompileReport, MatGraphView, MaterialDocument, MaterialError, MaterialPreviewSettings,
};
use redox_matgraph::NodeId;

/// One open material graph tab in the editor.
#[derive(Clone, Debug)]
pub struct MaterialEditorSession {
    /// On-disk document and dirty flag.
    pub document: MaterialDocument,
    /// Editable view (layout + undo).
    pub view: MatGraphView,
    /// Latest compile diagnostics.
    pub compile: CompileReport,
    /// Node highlighted from the error list.
    pub focused_node: Option<NodeId>,
    /// Preview options.
    pub preview: MaterialPreviewSettings,
    /// Last successful WGSL shown read-only in the UI.
    pub wgsl_display: String,
    /// Bumped when the graph changes so the render host can hot-reload preview variants.
    pub preview_revision: u64,
}

impl MaterialEditorSession {
    /// Opens a file into a new session.
    pub fn open(
        path: impl Into<PathBuf>,
        guid: Option<AssetGuid>,
    ) -> Result<Self, MaterialError> {
        let document = MaterialDocument::open(path, guid)?;
        let view = MatGraphView::from_graph(document.graph.clone());
        let compile = CompileReport::refresh(&document.graph);
        let wgsl_display = compile.wgsl.clone().unwrap_or_default();
        Ok(Self {
            document,
            view,
            compile,
            focused_node: None,
            preview: MaterialPreviewSettings::default(),
            wgsl_display,
            preview_revision: 0,
        })
    }

    /// Syncs the document graph from the view and refreshes compile state.
    pub fn sync_graph_from_view(&mut self) {
        self.document.graph = self.view.graph.clone();
        self.preview_revision = self.preview_revision.saturating_add(1);
        self.refresh_compile();
    }

    /// Re-runs validation/codegen diagnostics.
    pub fn refresh_compile(&mut self) {
        self.compile = CompileReport::refresh(&self.document.graph);
        if let Some(wgsl) = &self.compile.wgsl {
            self.wgsl_display = wgsl.clone();
        }
    }

    /// Saves when the graph compiles; returns the written path label.
    pub fn save(&mut self) -> Result<String, MaterialError> {
        self.document.graph = self.view.graph.clone();
        let compiled = self.document.save()?;
        self.wgsl_display = compiled.wgsl;
        self.refresh_compile();
        let label = self
            .document
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| self.document.graph.name.clone());
        Ok(label)
    }

    /// Title for the dock tab / window suffix.
    pub fn title(&self) -> String {
        let base = self
            .document
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or(&self.document.graph.name);
        if self.document.is_dirty() {
            format!("{base} *")
        } else {
            base.to_string()
        }
    }

    /// Parameter nodes `(id, name, default vec4)` for the params panel (asset-level only).
    pub fn param_nodes(&self) -> Vec<(NodeId, String, [f32; 4])> {
        use redox_matgraph::graph::NodeKind;
        self.document
            .graph
            .nodes
            .iter()
            .filter_map(|n| {
                if let NodeKind::Param {
                    name, default, ..
                } = &n.kind
                {
                    Some((n.id, name.clone(), *default))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Applies a parameter change and marks the document dirty.
    pub fn set_param(&mut self, id: NodeId, value: [f32; 4]) {
        self.view.set_param_default(id, value);
        self.sync_graph_from_view();
    }

    /// Focuses the node for message `index` in the error list.
    pub fn focus_error(&mut self, index: usize) {
        self.focused_node = self.compile.focus_node(index);
    }
}

/// Request to open a material from the asset browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialOpenRequest {
    /// Absolute path to `.matgraph`.
    pub path: PathBuf,
    /// Registry identity when known.
    pub guid: Option<AssetGuid>,
}
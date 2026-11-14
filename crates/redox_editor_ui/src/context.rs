//! Shared editor UI state passed into dock panels.

use parking_lot::Mutex;
use redox_editor_core::EditorContext;
use redox_editor_core::builtin_commands::record_spawn;
use redox_editor_debugger::{
    ConsoleFilter, CpuTimeline, FrameProfiler, GpuPassEntry, PieSession, RuntimeInspectorHighlight,
    RuntimeSearchQuery, TimeTravelBuffer,
};
use redox_editor_scene::{WorldGitTracker, WorldSession};
use redox_editor_viewport::{
    EditorCamera, GizmoState, ViewportDebugFlags, ViewportLayout, ViewportShading,
};
use std::sync::Arc;

use crate::asset_browser::{AssetBrowserPanel, AssetSpawnRequest};
use crate::audit_panel::AuditPanel;
use crate::import_dialog_panel::ImportDialogPanel;
use crate::import_log_panel::ImportLogPanel;
use crate::reference_panel::ReferenceViewerPanel;
use crate::brush_panel::BrushPanel;
use crate::command_palette::CommandPalette;
use crate::material_canvas::{focus_from_report, graph_from_view};
use crate::material_editor::{MaterialEditorSession, MaterialOpenRequest};
use crate::node_canvas::NodeCanvas;
use crate::outliner::{OutlinerAction, OutlinerPanel};
use crate::packager::PackagerPanel;
use crate::validation_model::ValidationPanelModel;
use redox_ecs::Entity;
use redox_gameplay::{AttributeSet, StateTreeComponent};
use std::path::PathBuf;

/// Everything the dock / menus need without cloning the whole editor.
pub struct EditorUiCtx {
    /// Headless editor state: world, registry, selection, undo, log, status and commands.
    pub core: EditorContext,
    /// Gizmo state.
    pub gizmo: Arc<Mutex<GizmoState>>,
    /// Editor camera.
    pub camera: Arc<Mutex<EditorCamera>>,
    /// Play-in-editor session.
    pub pie: Arc<Mutex<PieSession>>,
    /// Recent world snapshots for scrubbing back.
    pub time_travel: Arc<Mutex<TimeTravelBuffer>>,
    /// The open streaming world, if any.
    pub world_session: Arc<Mutex<Option<WorldSession>>>,
    /// Version-control status of the actors.
    pub git: Arc<Mutex<WorldGitTracker>>,
    /// Outliner panel state.
    pub outliner: Mutex<OutlinerPanel>,
    /// Asset browser panel state.
    pub assets: Mutex<AssetBrowserPanel>,
    /// Import log tab shell.
    pub import_log: Mutex<ImportLogPanel>,
    /// Import file dialog.
    pub import_dialog: Mutex<ImportDialogPanel>,
    /// Reference viewer tab.
    pub reference_viewer: Mutex<ReferenceViewerPanel>,
    /// Asset audit tab.
    pub audit: Mutex<AuditPanel>,
    /// Command palette state.
    pub palette: Mutex<CommandPalette>,
    /// Brush panel state.
    pub brush: Mutex<BrushPanel>,
    /// Node graph panel state.
    pub graph: Mutex<NodeCanvas>,
    /// Open material graph document, if any (Graph tab / Material mode).
    pub material: Mutex<Option<MaterialEditorSession>>,
    /// Packager panel state.
    pub packager: Mutex<PackagerPanel>,
    /// Entity driven by the node graph demo, if any.
    pub demo_agent: Mutex<Option<Entity>>,
    /// Actions requested by panels, executed by the host.
    pub pending_actions: Mutex<Vec<OutlinerAction>>,
    /// Asset browser Spawn clicks land here; integrator drains and places proxies.
    pub pending_asset_spawns: Mutex<Vec<AssetSpawnRequest>>,
    /// Whether placed objects snap to surfaces.
    pub surface_snap: Mutex<bool>,
    /// Whether gizmo axes follow the object's rotation.
    pub local_space: Mutex<bool>,
    /// Whether the previous committed transforms are drawn as ghosts.
    pub show_git_ghosts: Mutex<bool>,
    /// Shading mode of the viewport.
    pub shading: Mutex<ViewportShading>,
    /// Layout of the viewport (single or quad).
    pub layout: Mutex<ViewportLayout>,
    /// F6 overdraw / F7 AABBs — integrator reads these for debug draws.
    pub debug_flags: Mutex<ViewportDebugFlags>,
    /// Frame time history.
    pub profiler: Mutex<FrameProfiler>,
    /// Latest CPU span timeline from the dispatcher.
    pub cpu_timeline: Mutex<CpuTimeline>,
    /// Latest GPU pass list (durations optional).
    pub gpu_passes: Mutex<Vec<GpuPassEntry>>,
    /// Position of the time-travel scrubber.
    pub tt_scrub_idx: Mutex<usize>,
    /// Frames per second shown in the UI.
    pub fps: Mutex<f32>,
    /// Frame time in milliseconds shown in the UI.
    pub frame_ms: Mutex<f32>,
    /// Integrator may overwrite with real GPU memory stats; shown in Viewport toolbar.
    pub vram_label: Mutex<String>,
    /// Scene validation issues for the Issues tab.
    pub validation: Mutex<ValidationPanelModel>,
    /// Console tab filter state.
    pub console_filter: Mutex<ConsoleFilter>,
    /// Runtime inspector search box.
    pub runtime_search: Mutex<RuntimeSearchQuery>,
    /// Viewport highlight driven by runtime inspector.
    pub runtime_highlight: Mutex<RuntimeInspectorHighlight>,
    /// Commit message buffer for scene git (Console / git overlay).
    pub scene_commit_message: Mutex<String>,
}

impl EditorUiCtx {
    /// Creates the shared context from the editor's core state.
    pub fn new(
        core: EditorContext,
        gizmo: Arc<Mutex<GizmoState>>,
        camera: Arc<Mutex<EditorCamera>>,
    ) -> Self {
        core.log.info("RedOx Studio ready");
        let mut palette = CommandPalette::default();
        palette.add_registry_commands(core.commands.entries());
        Self {
            core,
            gizmo,
            camera,
            pie: Arc::new(Mutex::new(PieSession::new())),
            time_travel: Arc::new(Mutex::new(TimeTravelBuffer::default())),
            world_session: Arc::new(Mutex::new(None)),
            git: Arc::new(Mutex::new(WorldGitTracker::new("worlds/arena"))),
            outliner: Mutex::new(OutlinerPanel::default()),
            assets: Mutex::new(AssetBrowserPanel::default()),
            import_log: Mutex::new(ImportLogPanel::new()),
            import_dialog: Mutex::new(ImportDialogPanel::new()),
            reference_viewer: Mutex::new(ReferenceViewerPanel::new()),
            audit: Mutex::new(AuditPanel::new()),
            palette: Mutex::new(palette),
            brush: Mutex::new(BrushPanel::default()),
            graph: Mutex::new(NodeCanvas::default()),
            material: Mutex::new(None),
            packager: Mutex::new(PackagerPanel::new("worlds/arena")),
            demo_agent: Mutex::new(None),
            pending_actions: Mutex::new(Vec::new()),
            pending_asset_spawns: Mutex::new(Vec::new()),
            surface_snap: Mutex::new(true),
            local_space: Mutex::new(false),
            show_git_ghosts: Mutex::new(true),
            shading: Mutex::new(ViewportShading::Lit),
            layout: Mutex::new(ViewportLayout::Single),
            debug_flags: Mutex::new(ViewportDebugFlags::default()),
            profiler: Mutex::new(FrameProfiler::default()),
            cpu_timeline: Mutex::new(CpuTimeline::default()),
            gpu_passes: Mutex::new(Vec::new()),
            tt_scrub_idx: Mutex::new(0),
            fps: Mutex::new(0.0),
            frame_ms: Mutex::new(0.0),
            vram_label: Mutex::new("VRAM: n/a".into()),
            validation: Mutex::new(ValidationPanelModel::default()),
            console_filter: Mutex::new(ConsoleFilter::default()),
            runtime_search: Mutex::new(RuntimeSearchQuery::default()),
            runtime_highlight: Mutex::new(RuntimeInspectorHighlight::default()),
            scene_commit_message: Mutex::new("Update scene".into()),
        }
    }

    /// Appends a line to the console.
    pub fn log(&self, line: impl Into<String>) {
        self.core.log.push(line);
    }

    /// Registers the open scene and project root for scene-level git (status, diff, commit, pull).
    pub fn track_scene_git(&self, scene_file: impl AsRef<std::path::Path>, project_root: impl AsRef<std::path::Path>) {
        let mut git = self.git.lock();
        git.set_tracked_scene(scene_file, project_root);
        if let Err(err) = git.refresh_scene_git() {
            self.log(format!("[WARN] Scene git: {err}"));
        }
    }

    /// Queue an asset spawn request (no world mutation).
    pub fn request_asset_spawn(&self, path: impl Into<PathBuf>) {
        self.pending_asset_spawns
            .lock()
            .push(AssetSpawnRequest::new(path.into()));
    }

    /// Queue a fully-specified spawn request (path + optional label + kind).
    pub fn enqueue_asset_spawn(&self, req: AssetSpawnRequest) {
        self.pending_asset_spawns.lock().push(req);
    }

    /// Drain queued asset spawn requests for the integrator.
    pub fn drain_asset_spawns(&self) -> Vec<AssetSpawnRequest> {
        std::mem::take(&mut *self.pending_asset_spawns.lock())
    }

    /// Opens a `.matgraph` into the material editor and switches the graph canvas to Material mode.
    pub fn open_material(&self, request: MaterialOpenRequest) -> Result<(), String> {
        let session = MaterialEditorSession::open(&request.path, request.guid)
            .map_err(|e| e.to_string())?;
        let graph = graph_from_view(&session.view);
        let active =
            focus_from_report(&session.compile, session.focused_node);
        {
            let mut canvas = self.graph.lock();
            canvas.load_material(graph, active);
        }
        *self.material.lock() = Some(session);
        self.log(format!(
            "[INFO] Opened material {}",
            request.path.display()
        ));
        Ok(())
    }

    /// Pushes the material view back into the node canvas (after edits).
    pub fn refresh_material_canvas(&self) {
        let material = self.material.lock();
        let Some(session) = material.as_ref() else {
            return;
        };
        let graph = graph_from_view(&session.view);
        let active = focus_from_report(&session.compile, session.focused_node);
        drop(material);
        self.graph
            .lock()
            .load_material(graph, active);
    }

    /// Spawn a patrol/chase AI agent for the StateTree debugger.
    /// The spawn is one undo step named "Spawn StateTree Agent".
    pub fn spawn_demo_agent(&self) {
        self.core.undo.lock().begin("Spawn StateTree Agent");
        let e = {
            let mut world = self.core.world.lock();
            let e = world.spawn(); // undo: tx
            world.add_component( // undo: tx
                e,
                redox_math::Transform::from_translation(redox_math::Vec3::new(56.0, 1.0, 50.0)),
            );
            world.add_component(e, StateTreeComponent::patrol_chase()); // undo: tx
            world.add_component(e, AttributeSet::default_character()); // undo: tx
            e
        };
        record_spawn(&self.core, e);
        self.core.undo.lock().commit();
        *self.demo_agent.lock() = Some(e);
        self.graph.lock().load_patrol_chase();
        self.log(format!("[INFO] Demo StateTree agent #{}", e.id()));
    }

    /// Graph StateTree/GAS source: primary selection if it has `StateTreeComponent`, else `demo_agent`.
    pub fn resolve_graph_agent_entity(&self) -> Option<Entity> {
        let primary = self.core.selection.lock().primary;
        {
            let world = self.core.world.lock();
            if let Some(e) = primary
                && world.get_component::<StateTreeComponent>(e).is_some()
            {
                return Some(e);
            }
        }
        *self.demo_agent.lock()
    }
}

//! Shared editor UI state passed into dock panels.

use parking_lot::Mutex;
use redox_ecs::World;
use redox_editor_core::{
    EditorCamera, FrameProfiler, GizmoState, PieSession, Selection, TimeTravelBuffer, UndoStack,
    ViewportDebugFlags, ViewportLayout, ViewportShading, WorldGitTracker, WorldSession,
};
use redox_reflect::TypeRegistry;
use std::sync::Arc;

use crate::asset_browser::{AssetBrowserPanel, AssetSpawnRequest};
use crate::brush_panel::BrushPanel;
use crate::command_palette::CommandPalette;
use crate::node_canvas::NodeCanvas;
use crate::outliner::{OutlinerAction, OutlinerPanel};
use crate::packager::PackagerPanel;
use redox_ecs::Entity;
use redox_gameplay::{AttributeSet, StateTreeComponent};
use std::path::PathBuf;

/// Everything the dock / menus need without cloning the whole editor.
pub struct EditorUiCtx {
    pub world: Arc<Mutex<World>>,
    pub selection: Arc<Mutex<Selection>>,
    pub registry: Arc<Mutex<TypeRegistry>>,
    pub gizmo: Arc<Mutex<GizmoState>>,
    pub undo: Arc<Mutex<UndoStack>>,
    pub camera: Arc<Mutex<EditorCamera>>,
    pub pie: Arc<Mutex<PieSession>>,
    pub time_travel: Arc<Mutex<TimeTravelBuffer>>,
    pub world_session: Arc<Mutex<Option<WorldSession>>>,
    pub git: Arc<Mutex<WorldGitTracker>>,
    pub outliner: Mutex<OutlinerPanel>,
    pub assets: Mutex<AssetBrowserPanel>,
    pub palette: Mutex<CommandPalette>,
    pub brush: Mutex<BrushPanel>,
    pub graph: Mutex<NodeCanvas>,
    pub packager: Mutex<PackagerPanel>,
    pub demo_agent: Mutex<Option<Entity>>,
    pub pending_actions: Mutex<Vec<OutlinerAction>>,
    /// Asset browser Spawn clicks land here; integrator drains and places proxies.
    pub pending_asset_spawns: Mutex<Vec<AssetSpawnRequest>>,
    pub surface_snap: Mutex<bool>,
    pub local_space: Mutex<bool>,
    pub show_git_ghosts: Mutex<bool>,
    pub shading: Mutex<ViewportShading>,
    pub layout: Mutex<ViewportLayout>,
    /// F6 overdraw / F7 AABBs — integrator reads these for debug draws.
    pub debug_flags: Mutex<ViewportDebugFlags>,
    pub profiler: Mutex<FrameProfiler>,
    pub tt_scrub_idx: Mutex<usize>,
    pub fps: Mutex<f32>,
    pub frame_ms: Mutex<f32>,
    pub console: Mutex<Vec<String>>,
    pub status: Mutex<String>,
    /// Integrator may overwrite with real GPU memory stats; shown in Viewport toolbar.
    pub vram_label: Mutex<String>,
}

impl EditorUiCtx {
    pub fn new(
        world: Arc<Mutex<World>>,
        selection: Arc<Mutex<Selection>>,
        registry: Arc<Mutex<TypeRegistry>>,
        gizmo: Arc<Mutex<GizmoState>>,
        undo: Arc<Mutex<UndoStack>>,
        camera: Arc<Mutex<EditorCamera>>,
    ) -> Self {
        Self {
            world,
            selection,
            registry,
            gizmo,
            undo,
            camera,
            pie: Arc::new(Mutex::new(PieSession::new())),
            time_travel: Arc::new(Mutex::new(TimeTravelBuffer::default())),
            world_session: Arc::new(Mutex::new(None)),
            git: Arc::new(Mutex::new(WorldGitTracker::new("worlds/arena"))),
            outliner: Mutex::new(OutlinerPanel::default()),
            assets: Mutex::new(AssetBrowserPanel::default()),
            palette: Mutex::new(CommandPalette::default()),
            brush: Mutex::new(BrushPanel::default()),
            graph: Mutex::new(NodeCanvas::default()),
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
            tt_scrub_idx: Mutex::new(0),
            fps: Mutex::new(0.0),
            frame_ms: Mutex::new(0.0),
            console: Mutex::new(vec!["[INFO] RedOx Studio ready".into()]),
            status: Mutex::new(String::new()),
            vram_label: Mutex::new("VRAM: n/a".into()),
        }
    }

    pub fn log(&self, line: impl Into<String>) {
        let mut c = self.console.lock();
        c.push(line.into());
        if c.len() > 200 {
            c.remove(0);
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

    /// Spawn a patrol/chase AI agent for the StateTree debugger.
    pub fn spawn_demo_agent(&self) {
        let mut world = self.world.lock();
        let e = world.spawn();
        world.add_component(
            e,
            redox_math::Transform::from_translation(redox_math::Vec3::new(56.0, 1.0, 50.0)),
        );
        world.add_component(e, StateTreeComponent::patrol_chase());
        world.add_component(e, AttributeSet::default_character());
        *self.demo_agent.lock() = Some(e);
        self.graph.lock().load_patrol_chase();
        self.log(format!("[INFO] Demo StateTree agent #{}", e.id()));
    }

    /// Graph StateTree/GAS source: primary selection if it has `StateTreeComponent`, else `demo_agent`.
    pub fn resolve_graph_agent_entity(&self) -> Option<Entity> {
        let primary = self.selection.lock().primary;
        {
            let world = self.world.lock();
            if let Some(e) = primary {
                if world.get_component::<StateTreeComponent>(e).is_some() {
                    return Some(e);
                }
            }
        }
        *self.demo_agent.lock()
    }
}

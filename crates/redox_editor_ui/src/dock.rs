use egui_dock::{DockArea, DockState, Style, TabViewer};
use parking_lot::Mutex;
use redox_editor_core::EditorLog;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::context::EditorUiCtx;
use crate::git_overlay::GitOverlay;
use crate::inspector::{InspectorPanel, selection_entities};
use crate::profiler_panel::ProfilerPanel;
use crate::undo_history::UndoHistoryPanel;
use crate::validation_panel::ValidationPanel;
use redox_editor_assets::AssetService;
use redox_editor_viewport::{GizmoMode, ViewportLayout, ViewportShading};

/// Tabs of the editor dock.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    /// The 3D viewport.
    Viewport,
    /// World outliner.
    Outliner,
    /// Component inspector.
    Inspector,
    /// Asset browser.
    AssetBrowser,
    /// Console.
    Console,
    /// Node graph editor.
    Graph,
    /// Time-travel scrubber.
    TimeTravel,
    /// Frame profiler.
    Profiler,
    /// Project packager.
    Packager,
    /// Undo history: the steps, the current position and jumps to any step.
    UndoHistory,
    /// Scene validation issues.
    Issues,
    /// Import queue log.
    ImportLog,
    /// Reference viewer for the selected asset.
    ReferenceViewer,
    /// Asset audit and size map export.
    AssetAudit,
}

impl std::fmt::Display for Tab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tab::Viewport => write!(f, "Viewport"),
            Tab::Outliner => write!(f, "Outliner"),
            Tab::Inspector => write!(f, "Inspector"),
            Tab::AssetBrowser => write!(f, "Assets"),
            Tab::Console => write!(f, "Console"),
            Tab::Graph => write!(f, "Graph"),
            Tab::TimeTravel => write!(f, "Time-Travel"),
            Tab::Profiler => write!(f, "Profiler"),
            Tab::Packager => write!(f, "Packager"),
            Tab::UndoHistory => write!(f, "Undo History"),
            Tab::Issues => write!(f, "Issues"),
            Tab::ImportLog => write!(f, "Import Log"),
            Tab::ReferenceViewer => write!(f, "References"),
            Tab::AssetAudit => write!(f, "Asset Audit"),
        }
    }
}

/// The dock layout of the editor.
pub struct EditorDock {
    /// Layout tree of the tabs.
    pub tree: DockState<Tab>,
}

impl Default for EditorDock {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorDock {
    /// Tabs every layout must expose (old `.ron` files may omit newer ones).
    pub const REQUIRED_TABS: &'static [Tab] = &[
        Tab::Viewport,
        Tab::Outliner,
        Tab::Inspector,
        Tab::AssetBrowser,
        Tab::Console,
        Tab::Graph,
        Tab::TimeTravel,
        Tab::Profiler,
        Tab::Packager,
        Tab::UndoHistory,
        Tab::Issues,
        Tab::ImportLog,
        Tab::ReferenceViewer,
        Tab::AssetAudit,
    ];

    /// Creates the dock with the layout from `editor_layout.ron` in the working directory.
    pub fn new() -> Self {
        Self::load_or_default(Path::new("editor_layout.ron"), &EditorLog::new())
    }

    /// Loads the layout at `path`. A missing file gives the default layout. A file that does not
    /// parse is logged, renamed to `<name>.broken` and replaced by the default layout (the editor
    /// always opens).
    pub fn load_or_default(path: &Path, log: &EditorLog) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    log.error(format!("cannot read {}: {err}", path.display()));
                }
                return Self {
                    tree: Self::default_tree(),
                };
            }
        };
        match ron::from_str::<DockState<Tab>>(&text) {
            Ok(tree) => {
                let mut dock = Self { tree };
                dock.ensure_tabs();
                dock
            }
            Err(err) => {
                log.error(format!(
                    "{} does not parse ({err}); the default layout is used",
                    path.display()
                ));
                let broken = broken_path(path);
                match std::fs::rename(path, &broken) {
                    Ok(()) => log.info(format!("damaged layout moved to {}", broken.display())),
                    Err(rename_err) => log.error(format!(
                        "cannot move the damaged layout to {}: {rename_err}",
                        broken.display()
                    )),
                }
                Self {
                    tree: Self::default_tree(),
                }
            }
        }
    }

    /// The default layout.
    pub fn default_tree() -> DockState<Tab> {
        let mut tree = DockState::new(vec![Tab::Viewport]);
        let [_a, _b] = tree.main_surface_mut().split_left(
            egui_dock::NodeIndex::root(),
            0.2,
            vec![Tab::Outliner],
        );
        let [_c, _d] = tree.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.25,
            vec![Tab::Inspector],
        );
        tree.main_surface_mut().split_below(
            egui_dock::NodeIndex::root(),
            0.72,
            vec![
                Tab::AssetBrowser,
                Tab::ImportLog,
                Tab::ReferenceViewer,
                Tab::AssetAudit,
                Tab::Console,
                Tab::Graph,
                Tab::TimeTravel,
                Tab::Profiler,
                Tab::Packager,
                Tab::UndoHistory,
                Tab::Issues,
            ],
        );
        tree
    }

    /// Restores the default layout.
    pub fn reset_layout(&mut self) {
        self.tree = Self::default_tree();
    }

    /// Merge-add any `REQUIRED_TABS` missing from a loaded layout (e.g. Packager/Profiler).
    pub fn ensure_tabs(&mut self) {
        for tab in Self::REQUIRED_TABS {
            if self.tree.find_tab(tab).is_some() {
                continue;
            }
            // Prefer the bottom strip (Assets / Console / Graph / Time-Travel).
            let anchor = [Tab::AssetBrowser, Tab::Console, Tab::Graph, Tab::TimeTravel]
                .into_iter()
                .find_map(|t| self.tree.find_tab(&t));
            if let Some(path) = anchor {
                self.tree[path.surface][path.node].append_tab(tab.clone());
            } else {
                self.tree.push_to_first_leaf(tab.clone());
            }
        }
    }

    /// Draws the dock into `ui`; the viewport tab shows `viewport_tex_id` and reports its rectangle
    /// through `viewport_rect`.
    pub fn show_with_ui(
        &mut self,
        ui: &mut egui::Ui,
        ui_ctx: &EditorUiCtx,
        viewport_tex_id: Option<egui::TextureId>,
        viewport_rect: Option<Arc<Mutex<Option<egui::Rect>>>>,
        assets: Option<&mut AssetService>,
    ) {
        let mut viewer = TabViewerImpl {
            ui_ctx,
            viewport_tex_id,
            viewport_rect,
            assets,
        };
        let style = Style::from_egui(ui.style().as_ref());
        DockArea::new(&mut self.tree)
            .style(style)
            .show_inside(ui, &mut viewer);
    }

    /// Saves the layout as RON, atomically.
    pub fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let s = ron::ser::to_string_pretty(&self.tree, ron::ser::PrettyConfig::default())?;
        redox_scene::ofpa::write_atomic(path, s.as_bytes())?;
        Ok(())
    }

    /// Loads a layout saved by [`Self::save_to_file`] and adds tabs that are missing.
    pub fn load_from_file(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let s = std::fs::read_to_string(path)?;
        self.tree = ron::from_str(&s)?;
        self.ensure_tabs();
        Ok(())
    }

    /// Serializes the layout to a RON string.
    pub fn save_to_string(&self) -> String {
        ron::ser::to_string(&self.tree).unwrap_or_default()
    }

    /// Loads a layout from a RON string; invalid input keeps the current layout.
    pub fn load_from_string(&mut self, s: &str) {
        if let Ok(tree) = ron::from_str(s) {
            self.tree = tree;
            self.ensure_tabs();
        }
    }

    /// Back-compat shim.
    pub fn show(&mut self, ctx: &egui::Context) {
        let _ = ctx;
    }

    /// Does nothing; kept for callers of the old windowed API.
    pub fn show_with_viewport(
        &mut self,
        ctx: &egui::Context,
        _viewport_tex_id: Option<egui::TextureId>,
        _viewport_rect: Option<Arc<Mutex<Option<egui::Rect>>>>,
    ) {
        let _ = ctx;
    }
}

/// Where a damaged layout is moved: the same path with `.broken` appended.
fn broken_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".broken");
    PathBuf::from(name)
}

struct TabViewerImpl<'a> {
    ui_ctx: &'a EditorUiCtx,
    viewport_tex_id: Option<egui::TextureId>,
    viewport_rect: Option<Arc<Mutex<Option<egui::Rect>>>>,
    assets: Option<&'a mut AssetService>,
}

impl TabViewer for TabViewerImpl<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        format!("{tab}").into()
    }

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(("editor_dock_tab", tab.to_string()))
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Tab::Viewport => {
                ui.horizontal(|ui| {
                    let mut g = self.ui_ctx.gizmo.lock();
                    if ui
                        .selectable_label(g.mode == GizmoMode::Translate, "Translate (W)")
                        .clicked()
                    {
                        g.mode = GizmoMode::Translate;
                    }
                    if ui
                        .selectable_label(g.mode == GizmoMode::Rotate, "Rotate (E)")
                        .clicked()
                    {
                        g.mode = GizmoMode::Rotate;
                    }
                    if ui
                        .selectable_label(g.mode == GizmoMode::Scale, "Scale (R)")
                        .clicked()
                    {
                        g.mode = GizmoMode::Scale;
                    }
                    drop(g);
                    ui.separator();
                    {
                        let mut shading = self.ui_ctx.shading.lock();
                        for mode in [
                            ViewportShading::Lit,
                            ViewportShading::Unlit,
                            ViewportShading::Normals,
                            ViewportShading::Wireframe,
                            ViewportShading::Roughness,
                            ViewportShading::Depth,
                        ] {
                            if ui
                                .selectable_label(*shading == mode, mode.label())
                                .clicked()
                            {
                                *shading = mode;
                            }
                        }
                    }
                    ui.separator();
                    {
                        let mut layout = self.ui_ctx.layout.lock();
                        let label = match *layout {
                            ViewportLayout::Single => "Quad View",
                            ViewportLayout::Quad => "Single View",
                        };
                        if ui.button(label).clicked() {
                            *layout = layout.toggle();
                        }
                    }
                    let mut snap = self.ui_ctx.surface_snap.lock();
                    ui.checkbox(&mut snap, "Surface Snap");
                    let mut ghosts = self.ui_ctx.show_git_ghosts.lock();
                    ui.checkbox(&mut ghosts, "Git Ghosts");
                    drop(ghosts);
                    {
                        let mut dbg = self.ui_ctx.debug_flags.lock();
                        ui.checkbox(&mut dbg.show_overdraw, "Overdraw (F6)");
                        ui.checkbox(&mut dbg.show_aabbs, "AABBs (F7)");
                    }
                    self.ui_ctx.brush.lock().show_toolbar(ui);
                    if ui.button("Screenshot").clicked() {
                        *self.ui_ctx.core.status.lock() = "viewport_screenshot".into();
                    }
                    ui.label("View toggles (palette): Sky / GI / Fog / SDF");
                    let cam = self.ui_ctx.camera.lock();
                    ui.label(format!("Cam {:.0}×", cam.speed / 8.0));
                    ui.separator();
                    ui.weak(self.ui_ctx.vram_label.lock().as_str());
                });
                ui.separator();
                if let Some(tex_id) = self.viewport_tex_id {
                    let avail = ui.available_size();
                    let size = egui::vec2(avail.x, avail.y.max(1.0));
                    let resp = ui.image(egui::load::SizedTexture { id: tex_id, size });
                    if let Some(rect_arc) = &self.viewport_rect {
                        *rect_arc.lock() = Some(resp.rect);
                    }
                } else {
                    ui.colored_label(egui::Color32::GRAY, "Starting renderer…");
                    ui.spinner();
                }
            }
            Tab::Outliner => {
                let mut outliner = self.ui_ctx.outliner.lock();
                let world = self.ui_ctx.core.world.lock();
                let registry = self.ui_ctx.core.registry.lock();
                let mut selection = self.ui_ctx.core.selection.lock();
                let mut undo = self.ui_ctx.core.undo.lock();
                let session = self.ui_ctx.world_session.lock();
                let git = self.ui_ctx.git.lock();
                let scene_mode = session.is_none();
                let actions = outliner.show(
                    ui,
                    &world,
                    &registry,
                    &mut selection,
                    &mut undo,
                    &self.ui_ctx.core,
                    scene_mode,
                    session.as_ref(),
                    Some(&git),
                );
                drop(git);
                drop(session);
                drop(undo);
                drop(selection);
                drop(registry);
                drop(world);
                drop(outliner);
                if !actions.is_empty() {
                    self.ui_ctx.pending_actions.lock().extend(actions);
                }
            }
            Tab::Inspector => {
                let mut world = self.ui_ctx.core.world.lock();
                let registry = self.ui_ctx.core.registry.lock();
                let selection = self.ui_ctx.core.selection.lock();
                let mut undo = self.ui_ctx.core.undo.lock();
                let entities = selection_entities(selection.primary, &selection.set);
                InspectorPanel::show(
                    ui,
                    &registry,
                    &entities,
                    &mut world,
                    &mut undo,
                    &self.ui_ctx.core.log,
                );
            }
            Tab::UndoHistory => UndoHistoryPanel::show(ui, &self.ui_ctx.core),
            Tab::AssetBrowser => {
                let mut panel = self.ui_ctx.assets.lock();
                if let Some(req) = panel.show(ui) {
                    self.ui_ctx.pending_asset_spawns.lock().push(req);
                }
            }
            Tab::ImportLog => {
                let Some(service) = self.assets.as_mut() else {
                    ui.label("Assets are not loaded.");
                    return;
                };
                let mut shell = self.ui_ctx.import_log.lock();
                if let Some(path) = self.ui_ctx.assets.lock().take_import_log_focus() {
                    shell.focus(path);
                }
                if let Some(retry) =
                    shell.show(ui, service.import_log_mut())
                {
                    let _ = service.retry_import(&retry);
                }
            }
            Tab::ReferenceViewer => {
                let Some(service) = self.assets.as_mut() else {
                    ui.label("Assets are not loaded.");
                    return;
                };
                let guid = self.ui_ctx.assets.lock().model.selected().map(|e| e.guid);
                if let Some(guid) = guid {
                    self.ui_ctx.reference_viewer.lock().select(
                        guid,
                        service.registry(),
                        service.references_index(),
                    );
                }
                self.ui_ctx.reference_viewer.lock().show(ui);
            }
            Tab::AssetAudit => {
                let Some(service) = self.assets.as_mut() else {
                    ui.label("Assets are not loaded.");
                    return;
                };
                self.ui_ctx
                    .audit
                    .lock()
                    .show(ui, service.registry());
            }
            Tab::Console => {
                if GitOverlay::show(ui, &self.ui_ctx.git.lock()) {
                    *self.ui_ctx.core.status.lock() = "git_refresh".into();
                }
                ui.separator();
                crate::console_panel::ConsolePanel::show(ui, &self.ui_ctx, &self.ui_ctx.core);
            }
            Tab::Graph => {
                if let Some(mut session) = self.ui_ctx.material.lock().take() {
                    use crate::material_panel::{MaterialPanel, MaterialPanelAction};
                    match MaterialPanel::show(ui, &mut session) {
                        MaterialPanelAction::None | MaterialPanelAction::FocusNode => {}
                        MaterialPanelAction::Undo => {
                            if let Some(prev) = session.view.undo.undo(&session.view.graph) {
                                session.view.graph = prev;
                                session.sync_graph_from_view();
                            }
                        }
                        MaterialPanelAction::Save => {
                            match session.save() {
                                Ok(path) => {
                                    self.ui_ctx
                                        .log(format!("[INFO] Saved material {path}"));
                                }
                                Err(err) => {
                                    self.ui_ctx
                                        .log(format!("[ERROR] Material save failed: {err}"));
                                }
                            }
                        }
                    }
                    let graph = crate::material_canvas::graph_from_view(&session.view);
                    let active = crate::material_canvas::focus_from_report(
                        &session.compile,
                        session.focused_node,
                    );
                    self.ui_ctx
                        .graph
                        .lock()
                        .load_material(graph, active);
                    *self.ui_ctx.material.lock() = Some(session);
                }
                let agent_e = self.ui_ctx.resolve_graph_agent_entity();
                let primary = self.ui_ctx.core.selection.lock().primary;
                let mut world = self.ui_ctx.core.world.lock();
                let attrs = agent_e.and_then(|e| {
                    world
                        .get_component::<redox_gameplay::AttributeSet>(e)
                        .cloned()
                });
                let fps = (*self.ui_ctx.fps.lock()).max(1.0);
                let dt = 1.0 / fps;
                let mut graph = self.ui_ctx.graph.lock();
                use crate::node_canvas::GraphMode;
                // Avoid dual mut borrows from the same World: StateTree vs SkinnedMesh.
                if graph.mode == GraphMode::Material {
                    let mut session = self.ui_ctx.material.lock();
                    if let Some(mat) = session.as_mut() {
                        let (rect, response) =
                            ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
                        graph.show_material_canvas(
                            ui,
                            rect,
                            &response,
                            &mut mat.view,
                        );
                        mat.sync_graph_from_view();
                    } else {
                        graph.show(ui, None, None, None, None, dt);
                    }
                } else if graph.mode == GraphMode::AnimGraph {
                    let active_name = graph
                        .rig
                        .clips
                        .get(graph.active_clip)
                        .map(|c| c.name.clone());
                    let skinned_time = primary.and_then(|e| {
                        world
                            .get_component_mut::<redox_anim::SkinnedMesh>(e)
                            .map(|s| {
                                if let Some(name) = active_name.as_ref() {
                                    s.clip = name.clone();
                                }
                                &mut s.time
                            })
                    });
                    graph.show(ui, agent_e, None, attrs.as_ref(), skinned_time, dt);
                } else {
                    let agent = agent_e.and_then(|e| {
                        world.get_component_mut::<redox_gameplay::StateTreeComponent>(e)
                    });
                    graph.show(ui, agent_e, agent, attrs.as_ref(), None, dt);
                }
            }
            Tab::TimeTravel => {
                let tt = self.ui_ctx.time_travel.lock();
                let pie = self.ui_ctx.pie.lock();
                let len = tt.len();
                ui.label(format!("Frames buffered: {len} / {}", tt.capacity));
                ui.label(format!(
                    "PIE: running={} paused={}",
                    pie.running, pie.paused
                ));
                ui.separator();
                if len == 0 {
                    ui.label("Play PIE to record frames. Scrub when paused.");
                } else {
                    let mut idx = *self.ui_ctx.tt_scrub_idx.lock();
                    idx = idx.min(len - 1);
                    ui.horizontal(|ui| {
                        if ui.button("◀◀").clicked() {
                            idx = 0;
                        }
                        if ui.button("◀ Step").clicked() {
                            idx = idx.saturating_sub(1);
                        }
                        ui.label(format!("Frame {idx}"));
                        if ui.button("Step ▶").clicked() {
                            idx = (idx + 1).min(len - 1);
                        }
                        if ui.button("▶▶").clicked() {
                            idx = len - 1;
                        }
                    });
                    if ui
                        .add(egui::Slider::new(&mut idx, 0..=len.saturating_sub(1)).text("Scrub"))
                        .changed()
                        || ui.input(|i| {
                            i.key_pressed(egui::Key::ArrowLeft)
                                || i.key_pressed(egui::Key::ArrowRight)
                        })
                    {
                        *self.ui_ctx.core.status.lock() = format!("tt_scrub:{idx}");
                    }
                    *self.ui_ctx.tt_scrub_idx.lock() = idx;
                    let t_sec = idx as f32 / 60.0;
                    ui.label(format!("≈ {t_sec:.2}s @ 60fps assumed"));
                }
            }
            Tab::Profiler => {
                ProfilerPanel::show(
                    ui,
                    &self.ui_ctx.profiler.lock(),
                    &self.ui_ctx.cpu_timeline.lock(),
                    &self.ui_ctx.gpu_passes.lock(),
                );
            }
            Tab::Packager => {
                self.ui_ctx.packager.lock().show(ui);
            }
            Tab::Issues => {
                let mut model = self.ui_ctx.validation.lock();
                let mut selection = self.ui_ctx.core.selection.lock();
                let mut world = self.ui_ctx.core.world.lock();
                let registry = self.ui_ctx.core.registry.lock();
                let mut undo = self.ui_ctx.core.undo.lock();
                let (_actions, refresh) = ValidationPanel::show(
                    ui,
                    &mut model,
                    &mut selection,
                    &mut world,
                    &registry,
                    &mut undo,
                );
                if refresh {
                    *self.ui_ctx.core.status.lock() = "validation_refresh".into();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_layout_gives_the_default_layout_and_is_renamed_to_broken() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("redox_dock_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temporary folder is created");
        let path = dir.join("editor_layout.ron");
        std::fs::write(&path, "this is not a layout").expect("damaged layout is written");
        let log = EditorLog::new();

        let dock = EditorDock::load_or_default(&path, &log);

        for tab in EditorDock::REQUIRED_TABS {
            assert!(dock.tree.find_tab(tab).is_some(), "missing tab {tab}");
        }
        assert!(!path.exists());
        assert!(broken_path(&path).exists());
        assert!(log.lines().iter().any(|line| line.contains("[ERROR]")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dock_save_load_roundtrip() {
        let dock = EditorDock::new();
        let s = dock.save_to_string();
        assert!(!s.is_empty());
        let mut dock2 = EditorDock::new();
        dock2.load_from_string(&s);
        assert_eq!(
            dock.tree.iter_all_tabs().count(),
            dock2.tree.iter_all_tabs().count()
        );
    }

    #[test]
    fn ensure_tabs_adds_packager_profiler_graph() {
        // Old bottom strip: no Packager / Profiler / Graph.
        let mut tree = DockState::new(vec![Tab::Viewport]);
        let [_a, _b] = tree.main_surface_mut().split_left(
            egui_dock::NodeIndex::root(),
            0.2,
            vec![Tab::Outliner],
        );
        let [_c, _d] = tree.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.25,
            vec![Tab::Inspector],
        );
        tree.main_surface_mut().split_below(
            egui_dock::NodeIndex::root(),
            0.72,
            vec![Tab::AssetBrowser, Tab::Console, Tab::TimeTravel],
        );
        let mut dock = EditorDock { tree };
        assert!(dock.tree.find_tab(&Tab::Packager).is_none());
        assert!(dock.tree.find_tab(&Tab::Profiler).is_none());
        assert!(dock.tree.find_tab(&Tab::Graph).is_none());
        dock.ensure_tabs();
        for tab in EditorDock::REQUIRED_TABS {
            assert!(
                dock.tree.find_tab(tab).is_some(),
                "missing tab after ensure_tabs: {tab}"
            );
        }
    }
}

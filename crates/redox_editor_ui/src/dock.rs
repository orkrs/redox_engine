use egui_dock::{DockArea, DockState, Style, TabViewer};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::context::EditorUiCtx;
use crate::git_overlay::GitOverlay;
use crate::inspector::InspectorPanel;
use crate::profiler_panel::ProfilerPanel;
use redox_editor_core::{GizmoMode, ViewportLayout, ViewportShading};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Viewport,
    Outliner,
    Inspector,
    AssetBrowser,
    Console,
    Graph,
    TimeTravel,
    Profiler,
    Packager,
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
        }
    }
}

pub struct EditorDock {
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
    ];

    pub fn new() -> Self {
        let layout_path = std::path::Path::new("editor_layout.ron");
        if let Ok(s) = std::fs::read_to_string(layout_path)
            && let Ok(tree) = ron::from_str::<DockState<Tab>>(&s)
        {
            let mut dock = Self { tree };
            dock.ensure_tabs();
            return dock;
        }
        Self {
            tree: Self::default_tree(),
        }
    }

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
                Tab::Console,
                Tab::Graph,
                Tab::TimeTravel,
                Tab::Profiler,
                Tab::Packager,
            ],
        );
        tree
    }

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

    pub fn show_with_ui(
        &mut self,
        ui: &mut egui::Ui,
        ui_ctx: &EditorUiCtx,
        viewport_tex_id: Option<egui::TextureId>,
        viewport_rect: Option<Arc<Mutex<Option<egui::Rect>>>>,
    ) {
        let mut viewer = TabViewerImpl {
            ui_ctx,
            viewport_tex_id,
            viewport_rect,
        };
        let style = Style::from_egui(ui.style().as_ref());
        DockArea::new(&mut self.tree)
            .style(style)
            .show_inside(ui, &mut viewer);
    }

    pub fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let s = ron::ser::to_string_pretty(&self.tree, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, s)?;
        Ok(())
    }

    pub fn load_from_file(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let s = std::fs::read_to_string(path)?;
        self.tree = ron::from_str(&s)?;
        self.ensure_tabs();
        Ok(())
    }

    pub fn save_to_string(&self) -> String {
        ron::ser::to_string(&self.tree).unwrap_or_default()
    }

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

    pub fn show_with_viewport(
        &mut self,
        ctx: &egui::Context,
        _viewport_tex_id: Option<egui::TextureId>,
        _viewport_rect: Option<Arc<Mutex<Option<egui::Rect>>>>,
    ) {
        let _ = ctx;
    }
}

struct TabViewerImpl<'a> {
    ui_ctx: &'a EditorUiCtx,
    viewport_tex_id: Option<egui::TextureId>,
    viewport_rect: Option<Arc<Mutex<Option<egui::Rect>>>>,
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
                let world = self.ui_ctx.world.lock();
                let mut selection = self.ui_ctx.selection.lock();
                let session = self.ui_ctx.world_session.lock();
                let git = self.ui_ctx.git.lock();
                let actions =
                    outliner.show(ui, &world, &mut selection, session.as_ref(), Some(&git));
                drop(git);
                drop(session);
                drop(selection);
                drop(world);
                drop(outliner);
                if !actions.is_empty() {
                    self.ui_ctx.pending_actions.lock().extend(actions);
                }
            }
            Tab::Inspector => {
                let mut world = self.ui_ctx.world.lock();
                let registry = self.ui_ctx.registry.lock();
                let selection = self.ui_ctx.selection.lock();
                let mut undo = self.ui_ctx.undo.lock();
                InspectorPanel::show(ui, &registry, selection.primary, &mut world, &mut undo);
            }
            Tab::AssetBrowser => {
                let mut assets = self.ui_ctx.assets.lock();
                if let Some(req) = assets.show(ui) {
                    self.ui_ctx.pending_asset_spawns.lock().push(req);
                }
            }
            Tab::Console => {
                if GitOverlay::show(ui, &self.ui_ctx.git.lock()) {
                    *self.ui_ctx.status.lock() = "git_refresh".into();
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in self.ui_ctx.console.lock().iter() {
                            ui.monospace(line);
                        }
                    });
            }
            Tab::Graph => {
                let agent_e = self.ui_ctx.resolve_graph_agent_entity();
                let primary = self.ui_ctx.selection.lock().primary;
                let mut world = self.ui_ctx.world.lock();
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
                if graph.mode == GraphMode::AnimGraph {
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
                        *self.ui_ctx.status.lock() = format!("tt_scrub:{idx}");
                    }
                    *self.ui_ctx.tt_scrub_idx.lock() = idx;
                    let t_sec = idx as f32 / 60.0;
                    ui.label(format!("≈ {t_sec:.2}s @ 60fps assumed"));
                }
            }
            Tab::Profiler => {
                ProfilerPanel::show(ui, &self.ui_ctx.profiler.lock());
            }
            Tab::Packager => {
                self.ui_ctx.packager.lock().show(ui);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

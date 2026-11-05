//! Asset browser panel: lists the registry's assets with their import status, filters and
//! searches them, and turns user actions into [`AssetRequest`]s for the editor's asset service.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use redox_asset::guid::AssetStatus;
use redox_asset::{AssetGuid, AssetRegistry, DependentRef};
pub use redox_editor_assets::AssetKind;
use redox_editor_assets::{
    AssetAction, BrowserEntry, BrowserModel, StatusFilter, actions_for,
};

/// Request from the browser for the editor's asset service; the app drains them each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetRequest {
    /// Rescan the assets root (Rescan/Repair).
    Rescan,
    /// Move or rename a file; both paths are relative to the root.
    Move {
        /// Current path.
        path: String,
        /// Target path.
        to: String,
    },
    /// Delete a file; without `force` a referenced file is not deleted.
    Delete {
        /// Path relative to the root.
        path: String,
        /// Delete even when scenes reference the file.
        force: bool,
    },
    /// Import the file again, even when it is up to date.
    Reimport {
        /// Path relative to the root.
        path: String,
    },
    /// Cancel a queued or running import.
    CancelImport {
        /// Path relative to the root.
        path: String,
    },
    /// Show the file in the system file manager.
    Reveal {
        /// Path relative to the root.
        path: String,
    },
    /// Rewrite the path hints of the closed scenes that reference the asset.
    UpdateHints {
        /// Identity of the moved asset.
        guid: AssetGuid,
    },
}

/// Request to create a proxy object in the world for an asset (the browser never mutates the world).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetSpawnRequest {
    /// File to spawn (absolute path).
    pub path: PathBuf,
    /// Display name (usually file stem); optional for callers that only have a path.
    pub label: Option<String>,
    /// Kind of the file.
    pub kind: AssetKind,
}

impl AssetSpawnRequest {
    /// Creates a request for `path`; the label is the file stem.
    pub fn new(path: PathBuf) -> Self {
        let kind = AssetKind::from_path(&path);
        let label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        Self { path, label, kind }
    }
}

/// CPU-side marker: render/main can resolve `path` into GPU mesh later.
#[derive(Clone, Debug)]
pub struct EditorMeshProxy {
    /// File the proxy refers to.
    pub path: PathBuf,
    /// Display name.
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
struct GltfMeta {
    nodes: usize,
    meshes: usize,
}

/// What the header of a `.ktx2` / `.basis` file says (read without decoding the pixels).
#[derive(Clone, Debug)]
struct TextureMeta {
    summary: String,
}

fn is_gpu_texture(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ktx2") | Some("basis")
    )
}

fn probe_texture(path: &Path) -> Result<TextureMeta, String> {
    use redox_asset::{AssetSource, DirSource, LoadOptions, StreamedTexture};
    let dir = path.parent().ok_or("no parent folder")?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("bad file name")?;
    let source: std::sync::Arc<dyn AssetSource> = std::sync::Arc::new(DirSource::new(dir));
    let texture =
        StreamedTexture::open(source, name, LoadOptions::default()).map_err(|e| e.to_string())?;
    let info = texture.info();
    let shape = match (info.is_cube(), info.layers > 1) {
        (true, true) => format!("cube array x{}", info.layers),
        (true, false) => "cube".to_string(),
        (false, true) => format!("array x{}", info.layers),
        (false, false) => "2D".to_string(),
    };
    Ok(TextureMeta {
        summary: format!(
            "{}x{} {:?} · {} · {} mips · {:.1} KiB decoded",
            info.width,
            info.height,
            info.format,
            shape,
            info.mips,
            info.bytes_from(0) as f64 / 1024.0
        ),
    })
}

/// Which dialog text box is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextMode {
    Rename,
    Move,
}

impl TextMode {
    fn title(self) -> &'static str {
        match self {
            Self::Rename => "Rename asset",
            Self::Move => "Move asset",
        }
    }

    /// Target path for the typed text: renaming keeps the folder.
    fn target(self, path: &str, text: &str) -> String {
        match self {
            Self::Move => text.trim().to_owned(),
            Self::Rename => match path.rsplit_once('/') {
                Some((folder, _)) => format!("{folder}/{}", text.trim()),
                None => text.trim().to_owned(),
            },
        }
    }
}

#[derive(Clone, Debug)]
enum Dialog {
    Text {
        mode: TextMode,
        path: String,
        text: String,
    },
    ForceDelete {
        path: String,
        dependents: Vec<DependentRef>,
    },
}

/// The asset browser. Holds the model and the open dialogs; never touches the registry itself.
#[derive(Default)]
pub struct AssetBrowserPanel {
    /// Entries, filters and selection.
    pub model: BrowserModel,
    requests: Vec<AssetRequest>,
    dialog: Option<Dialog>,
    hint_prompt: Option<(AssetGuid, Vec<String>)>,
    references_view: Option<AssetGuid>,
    error_view: Option<AssetGuid>,
    meta_cache: HashMap<PathBuf, Result<GltfMeta, String>>,
    texture_cache: HashMap<PathBuf, Result<TextureMeta, String>>,
}

impl AssetBrowserPanel {
    /// Takes the entries and references from the registry.
    pub fn refresh(&mut self, registry: &AssetRegistry) {
        self.model.refresh(registry);
    }

    /// Takes the requests made since the last call.
    pub fn take_requests(&mut self) -> Vec<AssetRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Opens the delete confirmation for a referenced asset (after a blocked delete).
    pub fn ask_force_delete(&mut self, path: String, dependents: Vec<DependentRef>) {
        self.dialog = Some(Dialog::ForceDelete { path, dependents });
    }

    /// Offers to rewrite the path hints of the scenes that refer to a moved asset.
    pub fn ask_hint_update(&mut self, guid: AssetGuid, scenes: Vec<String>) {
        if !scenes.is_empty() {
            self.hint_prompt = Some((guid, scenes));
        }
    }

    /// Renders the browser. Returns a spawn request when the user spawns a mesh.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<AssetSpawnRequest> {
        let mut spawn = None;
        self.show_toolbar(ui);
        ui.separator();

        let rows: Vec<BrowserEntry> = self.model.visible().into_iter().cloned().collect();
        let selected = self.model.selected().map(|e| e.guid);
        let mut clicked: Option<AssetGuid> = None;
        let mut chosen: Vec<(AssetAction, BrowserEntry)> = Vec::new();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in &rows {
                ui.horizontal(|ui| {
                    let (thumb, _) =
                        ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(thumb, 2.0, swatch(entry.kind));
                    let label = format!("{}  [{}]", entry.name, entry.kind.label());
                    let response = ui
                        .selectable_label(selected == Some(entry.guid), label)
                        .on_hover_text(format!("{}\n{}", entry.path, entry.guid));
                    if response.clicked() {
                        clicked = Some(entry.guid);
                    }
                    if let Some(status) = &entry.status {
                        ui.colored_label(status_color(status), status_text(Some(status)));
                    }
                    response.context_menu(|ui| {
                        for action in actions_for(entry) {
                            if ui.button(action_label(action)).clicked() {
                                chosen.push((action, entry.clone()));
                                ui.close();
                            }
                        }
                    });
                });
            }
        });
        if let Some(guid) = clicked {
            self.model.select(Some(guid));
        }

        let root = self.model.root().to_path_buf();
        for (action, entry) in chosen {
            if let Some(request) = self.perform(action, &entry, &root, ui) {
                spawn = Some(request);
            }
        }
        if let Some(request) = self.show_selected_details(ui, &root) {
            spawn = Some(request);
        }
        self.show_windows(ui.ctx());
        spawn
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("{} assets", self.model.entries().len()));
            ui.text_edit_singleline(&mut self.model.filter.text);
            egui::ComboBox::from_id_salt("asset_kind_filter")
                .selected_text(kind_label(self.model.filter.kind))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.model.filter.kind, None, "All kinds");
                    for kind in [
                        AssetKind::Mesh,
                        AssetKind::Texture,
                        AssetKind::Scene,
                        AssetKind::Other,
                    ] {
                        ui.selectable_value(&mut self.model.filter.kind, Some(kind), kind.label());
                    }
                });
            egui::ComboBox::from_id_salt("asset_status_filter")
                .selected_text(status_filter_label(self.model.filter.status))
                .show_ui(ui, |ui| {
                    for filter in [
                        StatusFilter::Any,
                        StatusFilter::Imported,
                        StatusFilter::Outdated,
                        StatusFilter::Importing,
                        StatusFilter::Failed,
                        StatusFilter::Missing,
                    ] {
                        ui.selectable_value(
                            &mut self.model.filter.status,
                            filter,
                            status_filter_label(filter),
                        );
                    }
                });
            if ui.button("Rescan").clicked() {
                self.requests.push(AssetRequest::Rescan);
            }
        });
    }

    /// Turns a context-menu choice into a request, a dialog, or a spawn.
    fn perform(
        &mut self,
        action: AssetAction,
        entry: &BrowserEntry,
        root: &Path,
        ui: &egui::Ui,
    ) -> Option<AssetSpawnRequest> {
        match action {
            AssetAction::Spawn => return Some(AssetSpawnRequest::new(root.join(&entry.path))),
            AssetAction::Rename => {
                self.dialog = Some(Dialog::Text {
                    mode: TextMode::Rename,
                    path: entry.path.clone(),
                    text: entry.name.clone(),
                });
            }
            AssetAction::Move => {
                self.dialog = Some(Dialog::Text {
                    mode: TextMode::Move,
                    path: entry.path.clone(),
                    text: entry.path.clone(),
                });
            }
            AssetAction::Delete => self.requests.push(AssetRequest::Delete {
                path: entry.path.clone(),
                force: false,
            }),
            AssetAction::Reimport => self.requests.push(AssetRequest::Reimport {
                path: entry.path.clone(),
            }),
            AssetAction::Cancel => self.requests.push(AssetRequest::CancelImport {
                path: entry.path.clone(),
            }),
            AssetAction::ShowError => self.error_view = Some(entry.guid),
            AssetAction::RevealInExplorer => self.requests.push(AssetRequest::Reveal {
                path: entry.path.clone(),
            }),
            AssetAction::ShowReferences => self.references_view = Some(entry.guid),
            AssetAction::CopyGuid => ui.ctx().copy_text(entry.guid.to_string()),
        }
        None
    }

    fn show_selected_details(&mut self, ui: &mut egui::Ui, root: &Path) -> Option<AssetSpawnRequest> {
        let entry = self.model.selected()?.clone();
        let mut spawn = None;
        ui.separator();
        ui.label(format!(
            "{} · {} · {}",
            entry.path,
            entry.guid,
            status_text(entry.status.as_ref())
        ));
        let abs = root.join(&entry.path);
        if entry.kind == AssetKind::Mesh {
            if ui.button("Spawn selected").clicked() {
                spawn = Some(AssetSpawnRequest::new(abs.clone()));
            }
            self.ensure_gltf_meta(&abs);
            match self.meta_cache.get(&abs) {
                Some(Ok(meta)) => {
                    ui.label(format!("glTF: {} nodes · {} meshes", meta.nodes, meta.meshes));
                }
                Some(Err(err)) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 120, 100),
                        format!("glTF load failed: {err}"),
                    );
                }
                None => {}
            }
        }
        if entry.kind == AssetKind::Texture && is_gpu_texture(&abs) {
            let probe = self
                .texture_cache
                .entry(abs.clone())
                .or_insert_with(|| probe_texture(&abs));
            match probe {
                Ok(meta) => {
                    ui.label(&meta.summary);
                }
                Err(err) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 120, 100),
                        format!("texture header unreadable: {err}"),
                    );
                }
            }
        }
        spawn
    }

    fn ensure_gltf_meta(&mut self, path: &Path) {
        if self.meta_cache.contains_key(path) {
            return;
        }
        let result = redox_asset::load_scene_from_path(path)
            .map(|scene| GltfMeta {
                nodes: scene.nodes.len(),
                meshes: scene.meshes.len(),
            })
            .map_err(|e| e.to_string());
        self.meta_cache.insert(path.to_path_buf(), result);
    }

    /// Dialogs and small windows: rename/move text, forced delete, hint update, references, error.
    fn show_windows(&mut self, ctx: &egui::Context) {
        self.show_text_or_delete_dialog(ctx);
        self.show_hint_prompt(ctx);
        self.show_references_window(ctx);
        self.show_error_window(ctx);
    }

    fn show_text_or_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        self.dialog = match dialog {
            Dialog::Text {
                mode,
                path,
                mut text,
            } => {
                let mut answer = None;
                show_window(ctx, mode.title(), |ui| {
                    ui.label(format!("Current path: {path}"));
                    ui.text_edit_singleline(&mut text);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            answer = Some(false);
                        }
                    });
                });
                match answer {
                    None => Some(Dialog::Text { mode, path, text }),
                    Some(true) => {
                        let to = mode.target(&path, &text);
                        self.requests.push(AssetRequest::Move { path, to });
                        None
                    }
                    Some(false) => None,
                }
            }
            Dialog::ForceDelete { path, dependents } => {
                let mut answer = None;
                show_window(ctx, "Delete referenced asset?", |ui| {
                    ui.label(format!(
                        "{path} is referenced in {} place(s):",
                        dependents.len()
                    ));
                    for dep in &dependents {
                        ui.label(format!(
                            "{} · entity {} · {}",
                            dep.scene,
                            dep.entity.as_deref().unwrap_or("-"),
                            dep.component
                        ));
                    }
                    ui.label("Undo is not available for this delete. The references become missing.");
                    ui.horizontal(|ui| {
                        if ui.button("Delete anyway").clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            answer = Some(false);
                        }
                    });
                });
                match answer {
                    None => Some(Dialog::ForceDelete { path, dependents }),
                    Some(true) => {
                        self.requests.push(AssetRequest::Delete { path, force: true });
                        None
                    }
                    Some(false) => None,
                }
            }
        };
    }

    fn show_hint_prompt(&mut self, ctx: &egui::Context) {
        let Some((guid, scenes)) = self.hint_prompt.clone() else {
            return;
        };
        let mut answer = None;
        show_window(ctx, "Update path hints?", |ui| {
            ui.label(format!(
                "{} closed scene(s) keep the old path of this asset:",
                scenes.len()
            ));
            for scene in &scenes {
                ui.label(scene.as_str());
            }
            ui.horizontal(|ui| {
                if ui.button("Update").clicked() {
                    answer = Some(true);
                }
                if ui.button("Later").clicked() {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                self.requests.push(AssetRequest::UpdateHints { guid });
                self.hint_prompt = None;
            }
            Some(false) => self.hint_prompt = None,
            None => {}
        }
    }

    fn show_references_window(&mut self, ctx: &egui::Context) {
        let Some(guid) = self.references_view else {
            return;
        };
        let refs: Vec<DependentRef> = self
            .model
            .references_of(guid)
            .into_iter()
            .cloned()
            .collect();
        let title = self
            .model
            .entry(guid)
            .map_or_else(|| guid.to_string(), |e| e.path.clone());
        let mut close = false;
        show_window(ctx, &format!("References to {title}"), |ui| {
            if refs.is_empty() {
                ui.label("No scene refers to this asset.");
            }
            for r in &refs {
                ui.label(format!(
                    "{} · entity {} · {}",
                    r.scene,
                    r.entity.as_deref().unwrap_or("-"),
                    r.component
                ));
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
        if close {
            self.references_view = None;
        }
    }

    fn show_error_window(&mut self, ctx: &egui::Context) {
        let Some(guid) = self.error_view else {
            return;
        };
        let (path, message) = match self.model.entry(guid) {
            Some(entry) => (
                entry.path.clone(),
                match &entry.status {
                    Some(AssetStatus::Failed(message)) => message.clone(),
                    _ => "no error recorded".to_owned(),
                },
            ),
            None => (guid.to_string(), "asset is not registered".to_owned()),
        };
        let mut close = false;
        show_window(ctx, "Import error", |ui| {
            ui.label(&path);
            ui.colored_label(egui::Color32::from_rgb(220, 120, 100), message.as_str());
            if ui.button("Close").clicked() {
                close = true;
            }
        });
        if close {
            self.error_view = None;
        }
    }
}

fn show_window(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .show(ctx, add);
}

fn swatch(kind: AssetKind) -> egui::Color32 {
    match kind {
        AssetKind::Mesh => egui::Color32::from_rgb(90, 160, 220),
        AssetKind::Texture => egui::Color32::from_rgb(120, 190, 130),
        AssetKind::Scene => egui::Color32::from_rgb(210, 170, 90),
        AssetKind::Other => egui::Color32::from_rgb(140, 140, 150),
    }
}

fn status_color(status: &AssetStatus) -> egui::Color32 {
    match status {
        AssetStatus::Importing => egui::Color32::from_rgb(110, 170, 230),
        AssetStatus::Imported => egui::Color32::from_rgb(120, 190, 130),
        AssetStatus::Outdated => egui::Color32::from_rgb(220, 180, 90),
        AssetStatus::Failed(_) => egui::Color32::from_rgb(220, 120, 100),
        AssetStatus::Missing => egui::Color32::from_rgb(150, 150, 150),
    }
}

fn status_text(status: Option<&AssetStatus>) -> String {
    match status {
        None => "no import needed".to_owned(),
        Some(AssetStatus::Importing) => "importing".to_owned(),
        Some(AssetStatus::Imported) => "imported".to_owned(),
        Some(AssetStatus::Outdated) => "needs import".to_owned(),
        Some(AssetStatus::Failed(_)) => "import failed".to_owned(),
        Some(AssetStatus::Missing) => "missing".to_owned(),
    }
}

fn kind_label(kind: Option<AssetKind>) -> &'static str {
    kind.map_or("all kinds", AssetKind::label)
}

fn status_filter_label(filter: StatusFilter) -> &'static str {
    match filter {
        StatusFilter::Any => "any status",
        StatusFilter::Imported => "imported",
        StatusFilter::Outdated => "needs import",
        StatusFilter::Importing => "importing",
        StatusFilter::Failed => "failed",
        StatusFilter::Missing => "missing",
    }
}

fn action_label(action: AssetAction) -> &'static str {
    match action {
        AssetAction::Spawn => "Spawn",
        AssetAction::Rename => "Rename",
        AssetAction::Move => "Move",
        AssetAction::Delete => "Delete",
        AssetAction::Reimport => "Reimport",
        AssetAction::Cancel => "Cancel import",
        AssetAction::ShowError => "Show error",
        AssetAction::RevealInExplorer => "Reveal in Explorer",
        AssetAction::ShowReferences => "Show references",
        AssetAction::CopyGuid => "Copy GUID",
    }
}

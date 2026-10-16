use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Coarse asset classification used for thumbnails and spawn routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Mesh,
    Texture,
    Scene,
    Other,
}

impl AssetKind {
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("glb") | Some("gltf") => Self::Mesh,
            Some("png") | Some("jpg") | Some("jpeg") | Some("webp") | Some("hdr") | Some("ktx2") => {
                Self::Texture
            }
            Some("ron") => Self::Scene,
            _ => Self::Other,
        }
    }

    fn swatch(self) -> egui::Color32 {
        match self {
            Self::Mesh => egui::Color32::from_rgb(90, 160, 220),
            Self::Texture => egui::Color32::from_rgb(120, 190, 130),
            Self::Scene => egui::Color32::from_rgb(210, 170, 90),
            Self::Other => egui::Color32::from_rgb(140, 140, 150),
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Texture => "tex",
            Self::Scene => "scene",
            Self::Other => "file",
        }
    }
}

/// Request for the integrator / dock to spawn an asset proxy in the world.
/// Asset browser never mutates the ECS world itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetSpawnRequest {
    pub path: PathBuf,
    /// Display name (usually file stem); optional for callers that only have a path.
    pub label: Option<String>,
    pub kind: AssetKind,
}

impl AssetSpawnRequest {
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
    pub path: PathBuf,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
struct GltfMeta {
    nodes: usize,
    meshes: usize,
}

pub struct AssetBrowserPanel {
    pub current_dir: PathBuf,
    pub filter: String,
    /// Path selected for potential viewport drop.
    pub selected: Option<PathBuf>,
    /// Cached glTF probe results keyed by absolute-ish path string.
    meta_cache: HashMap<PathBuf, Result<GltfMeta, String>>,
}

impl Default for AssetBrowserPanel {
    fn default() -> Self {
        Self {
            current_dir: PathBuf::from("assets"),
            filter: String::new(),
            selected: None,
            meta_cache: HashMap::new(),
        }
    }
}

impl AssetBrowserPanel {
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

    /// Renders the browser. Returns a spawn request when the user clicks Spawn;
    /// callers should enqueue it on [`crate::context::EditorUiCtx::pending_asset_spawns`].
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<AssetSpawnRequest> {
        let mut spawn_req = None;
        ui.horizontal(|ui| {
            ui.label(format!("📁 {}", self.current_dir.display()));
            if ui.button("⬆ Up").clicked() {
                self.current_dir.pop();
            }
            ui.text_edit_singleline(&mut self.filter);
        });
        ui.separator();

        let filter = self.filter.to_lowercase();
        let mut entries = std::fs::read_dir(&self.current_dir)
            .map(|r| r.filter_map(|e| e.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        entries.sort_by_key(|e| {
            (
                !e.path().is_dir(),
                e.file_name().to_string_lossy().to_lowercase(),
            )
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in entries {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                    continue;
                }
                let is_dir = path.is_dir();
                let kind = if is_dir {
                    None
                } else {
                    Some(AssetKind::from_path(&path))
                };

                ui.horizontal(|ui| {
                    // Thumbnail placeholder: colored swatch by extension (dirs use folder tint).
                    let swatch = if is_dir {
                        egui::Color32::from_rgb(100, 110, 130)
                    } else {
                        kind.map(|k| k.swatch()).unwrap_or(egui::Color32::GRAY)
                    };
                    let (thumb, _) =
                        ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(thumb, 2.0, swatch);

                    let selected = self.selected.as_ref() == Some(&path);
                    let row_label = if is_dir {
                        format!("📁 {name}")
                    } else {
                        let tag = kind.map(|k| k.short_label()).unwrap_or("?");
                        format!("{name}  [{tag}]")
                    };
                    if ui.selectable_label(selected, row_label).clicked() {
                        if is_dir {
                            self.current_dir = path.clone();
                        } else {
                            self.selected = Some(path.clone());
                        }
                    }
                    if !is_dir && kind == Some(AssetKind::Mesh) {
                        if ui.small_button("Spawn").clicked() {
                            spawn_req = Some(AssetSpawnRequest::new(path.clone()));
                        }
                    }
                });
            }
        });

        ui.separator();
        if let Some(sel) = self.selected.clone() {
            let kind = AssetKind::from_path(&sel);
            ui.label(format!(
                "Selected: {} ({})",
                sel.display(),
                kind.short_label()
            ));
            if kind == AssetKind::Mesh {
                self.ensure_gltf_meta(&sel);
                match self.meta_cache.get(&sel) {
                    Some(Ok(meta)) => {
                        ui.label(format!(
                            "glTF: {} nodes · {} meshes",
                            meta.nodes, meta.meshes
                        ));
                    }
                    Some(Err(err)) => {
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 120, 100),
                            format!("glTF load failed: {err}"),
                        );
                    }
                    None => {}
                }
                if ui.button("Spawn selected").clicked() {
                    spawn_req = Some(AssetSpawnRequest::new(sel));
                }
            }
        } else {
            ui.label("Spawn queues Transform + EditorMeshProxy — render resolves later");
        }
        spawn_req
    }
}

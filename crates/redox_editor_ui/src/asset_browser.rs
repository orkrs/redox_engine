use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Coarse asset classification used for thumbnails and spawn routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
    /// A mesh or model file.
    Mesh,
    /// An image file.
    Texture,
    /// A scene file.
    Scene,
    /// Any other file.
    Other,
}

impl AssetKind {
    /// Classifies a file by its extension.
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("glb") | Some("gltf") => Self::Mesh,
            Some("png") | Some("jpg") | Some("jpeg") | Some("webp") | Some("hdr")
            | Some("ktx2") | Some("basis") => Self::Texture,
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

fn is_gpu_texture(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("ktx2") | Some("basis")
    )
}

/// Request for the integrator / dock to spawn an asset proxy in the world.
/// Asset browser never mutates the ECS world itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetSpawnRequest {
    /// File to spawn.
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

/// Panel that lists the files of the assets directory.
pub struct AssetBrowserPanel {
    /// Directory being shown.
    pub current_dir: PathBuf,
    /// Text filter for file names.
    pub filter: String,
    /// Path selected for potential viewport drop.
    pub selected: Option<PathBuf>,
    /// Cached glTF probe results keyed by absolute-ish path string.
    meta_cache: HashMap<PathBuf, Result<GltfMeta, String>>,
    /// Cached texture header probes.
    texture_cache: HashMap<PathBuf, Result<TextureMeta, String>>,
}

impl Default for AssetBrowserPanel {
    fn default() -> Self {
        Self {
            current_dir: PathBuf::from("assets"),
            filter: String::new(),
            selected: None,
            meta_cache: HashMap::new(),
            texture_cache: HashMap::new(),
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
                    if !is_dir
                        && kind == Some(AssetKind::Mesh)
                        && ui.small_button("Spawn").clicked()
                    {
                        spawn_req = Some(AssetSpawnRequest::new(path.clone()));
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
            if kind == AssetKind::Texture && is_gpu_texture(&sel) {
                let probe = self
                    .texture_cache
                    .entry(sel.clone())
                    .or_insert_with(|| probe_texture(&sel));
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

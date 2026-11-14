//! Import dialog model: file pick, metadata probe and queue confirmation (EDT-B4 step 2).

use std::path::{Path, PathBuf};

use redox_asset::AssetGuid;

use crate::browser_model::AssetKind;
use crate::import::{probe_texture_header};
use crate::import_settings::{ImportPresetId, PerAssetImportSettings, default_for_kind, settings_for_preset};

/// What the dialog can show as a visual hint before import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportPreview {
    /// Header probe for a texture file.
    TextureHeader {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Short format label.
        format: String,
    },
    /// glTF mesh silhouette needs B3; not available in this slice.
    MeshSilhouetteDeferred,
    /// No preview for this extension.
    None,
}

/// One row in the import dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDialogRow {
    /// Absolute path chosen by the user.
    pub source: PathBuf,
    /// Path relative to the assets root once copied or registered.
    pub rel_path: String,
    /// Classified kind.
    pub kind: AssetKind,
    /// Text from the probe step.
    pub probe_summary: String,
    /// Optional preview kind.
    pub preview: ImportPreview,
    /// Settings that will be written to `.meta` on confirm.
    pub settings: PerAssetImportSettings,
    /// Registry identity when the file is already known.
    pub guid: Option<AssetGuid>,
}

/// A job produced when the user confirms the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportDialogJob {
    /// Relative asset path.
    pub path: String,
    /// Identity when already registered.
    pub guid: Option<AssetGuid>,
    /// Serialised settings for the queue.
    pub settings: String,
}

/// Lifecycle of the dialog (UI-free).
#[derive(Clone, Debug, PartialEq)]
pub enum ImportDialogPhase {
    /// Waiting for file paths.
    Picking,
    /// Paths probed; user may edit settings.
    Preview(Vec<ImportDialogRow>),
    /// Jobs were handed to the queue.
    Committed(Vec<ImportDialogJob>),
    /// User cancelled before commit.
    Cancelled,
}

/// Pure model of the import dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDialogModel {
    /// Current phase.
    pub phase: ImportDialogPhase,
}

impl Default for ImportDialogModel {
    fn default() -> Self {
        Self {
            phase: ImportDialogPhase::Picking,
        }
    }
}

impl ImportDialogModel {
    /// Starts preview for absolute paths mapped under `assets_root`.
    pub fn begin_paths(
        paths: impl IntoIterator<Item = PathBuf>,
        assets_root: &Path,
    ) -> Self {
        let rows: Vec<ImportDialogRow> = paths
            .into_iter()
            .filter_map(|source| build_row(source, assets_root))
            .collect();
        let phase = if rows.is_empty() {
            ImportDialogPhase::Picking
        } else {
            ImportDialogPhase::Preview(rows)
        };
        Self { phase }
    }

    /// Changes the preset of one preview row.
    pub fn set_preset(&mut self, index: usize, preset: ImportPresetId) -> bool {
        let ImportDialogPhase::Preview(rows) = &mut self.phase else {
            return false;
        };
        let Some(row) = rows.get_mut(index) else {
            return false;
        };
        row.settings = settings_for_preset(preset);
        true
    }

    /// Merges edited settings into a row.
    pub fn update_settings(&mut self, index: usize, settings: PerAssetImportSettings) -> bool {
        let ImportDialogPhase::Preview(rows) = &mut self.phase else {
            return false;
        };
        rows.get_mut(index).map(|row| row.settings = settings).is_some()
    }

    /// Confirms and returns jobs for the import queue. Clears preview state.
    pub fn confirm(&mut self) -> Vec<ImportDialogJob> {
        let ImportDialogPhase::Preview(rows) = std::mem::replace(
            &mut self.phase,
            ImportDialogPhase::Picking,
        ) else {
            return Vec::new();
        };
        let jobs: Vec<ImportDialogJob> = rows
            .into_iter()
            .map(|row| ImportDialogJob {
                path: row.rel_path,
                guid: row.guid,
                settings: crate::import_settings::settings_blob(&row.settings),
            })
            .collect();
        self.phase = ImportDialogPhase::Committed(jobs.clone());
        jobs
    }

    /// Cancels before any job starts.
    pub fn cancel(&mut self) -> bool {
        match self.phase {
            ImportDialogPhase::Preview(_) => {
                self.phase = ImportDialogPhase::Cancelled;
                true
            }
            _ => false,
        }
    }

    /// Whether the dialog is still open for preview or picking.
    pub fn is_open(&self) -> bool {
        matches!(
            self.phase,
            ImportDialogPhase::Picking | ImportDialogPhase::Preview(_)
        )
    }
}

fn build_row(source: PathBuf, assets_root: &Path) -> Option<ImportDialogRow> {
    let rel = relative_under(assets_root, &source)?;
    let kind = AssetKind::from_path(&source);
    let (probe_summary, preview) = probe_file(&source, kind);
    Some(ImportDialogRow {
        source,
        rel_path: rel,
        kind,
        probe_summary,
        preview,
        settings: default_for_kind(kind),
        guid: None,
    })
}

fn relative_under(root: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn probe_file(path: &Path, kind: AssetKind) -> (String, ImportPreview) {
    match kind {
        AssetKind::Texture => match std::fs::read(path) {
            Ok(bytes) => match probe_texture_header(&bytes) {
                Ok(header) => {
                    let summary = format!("{}x{} {}", header.width, header.height, header.format);
                    let preview = ImportPreview::TextureHeader {
                        width: header.width,
                        height: header.height,
                        format: header.format,
                    };
                    (summary, preview)
                }
                Err(err) => (err, ImportPreview::None),
            },
            Err(err) => (err.to_string(), ImportPreview::None),
        },
        AssetKind::Mesh => (
            "mesh preview needs EDT-B3 (silhouette deferred)".into(),
            ImportPreview::MeshSilhouetteDeferred,
        ),
        _ => ("no import preview".into(), ImportPreview::None),
    }
}
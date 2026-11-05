//! Egui-free state of the asset browser: entries taken from the registry, filters, search,
//! selection, the actions allowed for an entry, and the references shown for an asset.

use std::path::{Path, PathBuf};

use redox_asset::guid::AssetStatus;
use redox_asset::{AssetGuid, AssetRegistry, DependentRef};

/// Coarse asset classification used for filters, thumbnails and spawn routing.
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

    /// Short label shown next to the name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Texture => "tex",
            Self::Scene => "scene",
            Self::Other => "file",
        }
    }
}

/// Status filter of the browser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusFilter {
    /// Every asset.
    #[default]
    Any,
    /// Imported with the current content.
    Imported,
    /// Changed or never imported.
    Outdated,
    /// Import running or queued.
    Importing,
    /// Last import failed.
    Failed,
    /// File is gone; references to it are missing.
    Missing,
}

impl StatusFilter {
    /// Whether an asset with `status` passes this filter.
    pub fn matches(self, status: Option<&AssetStatus>) -> bool {
        match (self, status) {
            (Self::Any, _) => true,
            (Self::Imported, Some(AssetStatus::Imported)) => true,
            (Self::Outdated, Some(AssetStatus::Outdated)) => true,
            (Self::Importing, Some(AssetStatus::Importing)) => true,
            (Self::Failed, Some(AssetStatus::Failed(_))) => true,
            (Self::Missing, Some(AssetStatus::Missing)) => true,
            _ => false,
        }
    }
}

/// Filters of the browser; all of them apply together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserFilter {
    /// Text searched in names and paths (case-insensitive substring).
    pub text: String,
    /// Only this kind, when set.
    pub kind: Option<AssetKind>,
    /// Only this status.
    pub status: StatusFilter,
}

/// One asset as listed by the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserEntry {
    /// Path relative to the assets root, `/`-separated.
    pub path: String,
    /// File name.
    pub name: String,
    /// Identity of the asset.
    pub guid: AssetGuid,
    /// Kind by extension.
    pub kind: AssetKind,
    /// Import status; `None` when the asset needs no import.
    pub status: Option<AssetStatus>,
}

/// An action offered for an asset in the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetAction {
    /// Create a proxy object in the world (meshes only).
    Spawn,
    /// Rename the file in its folder.
    Rename,
    /// Move the file to another path.
    Move,
    /// Delete the file (asks about references first).
    Delete,
    /// Import again, even when the asset is up to date.
    Reimport,
    /// Cancel a queued or running import.
    Cancel,
    /// Show the error of the last failed import.
    ShowError,
    /// Open in the system file manager.
    RevealInExplorer,
    /// List the scenes and entities that reference the asset.
    ShowReferences,
    /// Copy the GUID to the clipboard.
    CopyGuid,
}

/// Actions allowed for an entry. A missing file cannot be renamed, moved or revealed; a failed
/// import can be inspected and retried; a running import can be cancelled.
pub fn actions_for(entry: &BrowserEntry) -> Vec<AssetAction> {
    match entry.status {
        Some(AssetStatus::Missing) => vec![
            AssetAction::ShowReferences,
            AssetAction::CopyGuid,
            AssetAction::Delete,
        ],
        Some(AssetStatus::Failed(_)) => vec![
            AssetAction::ShowError,
            AssetAction::Reimport,
            AssetAction::Rename,
            AssetAction::Move,
            AssetAction::Delete,
            AssetAction::RevealInExplorer,
            AssetAction::ShowReferences,
            AssetAction::CopyGuid,
        ],
        Some(AssetStatus::Importing) => vec![
            AssetAction::Cancel,
            AssetAction::ShowReferences,
            AssetAction::CopyGuid,
        ],
        _ => {
            let mut actions = vec![
                AssetAction::Rename,
                AssetAction::Move,
                AssetAction::Delete,
                AssetAction::Reimport,
                AssetAction::RevealInExplorer,
                AssetAction::ShowReferences,
                AssetAction::CopyGuid,
            ];
            if entry.kind == AssetKind::Mesh {
                actions.insert(0, AssetAction::Spawn);
            }
            actions
        }
    }
}

/// The browser's state: entries, filters and selection, refreshed from the registry.
#[derive(Clone, Debug, Default)]
pub struct BrowserModel {
    root: PathBuf,
    entries: Vec<BrowserEntry>,
    references: Vec<DependentRef>,
    /// Filters applied to the entries.
    pub filter: BrowserFilter,
    selected: Option<AssetGuid>,
}

impl BrowserModel {
    /// Creates an empty model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the entries and references from the registry.
    pub fn refresh(&mut self, registry: &AssetRegistry) {
        self.root = registry.root().to_path_buf();
        self.entries = registry
            .iter()
            .map(|(path, guid)| BrowserEntry {
                path: path.to_owned(),
                name: file_name(path),
                guid,
                kind: AssetKind::from_path(Path::new(path)),
                status: registry.status(path).cloned(),
            })
            .collect();
        self.references = registry.scene_references();
        if let Some(selected) = self.selected
            && !self.entries.iter().any(|e| e.guid == selected)
        {
            self.selected = None;
        }
    }

    /// Assets root the entries are relative to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All entries, in path order.
    pub fn entries(&self) -> &[BrowserEntry] {
        &self.entries
    }

    /// Entries that pass the filters, in path order.
    pub fn visible(&self) -> Vec<&BrowserEntry> {
        let text = self.filter.text.to_lowercase();
        self.entries
            .iter()
            .filter(|e| self.filter.kind.is_none_or(|kind| e.kind == kind))
            .filter(|e| self.filter.status.matches(e.status.as_ref()))
            .filter(|e| {
                text.is_empty()
                    || e.name.to_lowercase().contains(&text)
                    || e.path.to_lowercase().contains(&text)
            })
            .collect()
    }

    /// Selects an asset (or clears the selection).
    pub fn select(&mut self, guid: Option<AssetGuid>) {
        self.selected = guid;
    }

    /// The selected entry, if it is still registered.
    pub fn selected(&self) -> Option<&BrowserEntry> {
        let guid = self.selected?;
        self.entry(guid)
    }

    /// The entry of an asset.
    pub fn entry(&self, guid: AssetGuid) -> Option<&BrowserEntry> {
        self.entries.iter().find(|e| e.guid == guid)
    }

    /// Scenes and entities that reference an asset, in scene path order.
    pub fn references_of(&self, guid: AssetGuid) -> Vec<&DependentRef> {
        self.references.iter().filter(|r| r.guid == guid).collect()
    }
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

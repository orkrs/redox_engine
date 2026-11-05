//! Stable asset identity: GUIDs stored in `.meta` sidecar files and a registry that maps
//! GUIDs to paths.
//!
//! Every asset file `a/b.png` gets a sidecar `a/b.png.meta` (RON) containing its
//! [`AssetGuid`]. Moving the pair keeps the GUID, so references by GUID survive renames
//! and reorganisation. Copying a file together with its sidecar would produce a duplicate
//! GUID; [`AssetRegistry::scan`] detects that and gives the later path a fresh GUID.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::hot_reload::AssetEvent;

/// Extension of sidecar files (without the dot).
pub const META_EXTENSION: &str = "meta";

/// 128-bit asset identifier, written as 32 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetGuid(pub u128);

impl AssetGuid {
    /// Generates a random (version 4) GUID.
    pub fn new_random() -> Self {
        Self(uuid::Uuid::new_v4().as_u128())
    }
}

impl fmt::Display for AssetGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl fmt::Debug for AssetGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetGuid({self})")
    }
}

/// A GUID string was not 32 hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid asset guid {0:?}: expected 32 hexadecimal digits")]
pub struct ParseGuidError(String);

impl FromStr for AssetGuid {
    type Err = ParseGuidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ParseGuidError(s.to_owned()));
        }
        u128::from_str_radix(s, 16)
            .map(Self)
            .map_err(|_| ParseGuidError(s.to_owned()))
    }
}

impl Serialize for AssetGuid {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AssetGuid {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Contents of a `.meta` sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetMeta {
    /// Identity of the asset.
    pub guid: AssetGuid,
    /// Version of the importer that produced derived data (bumped to force a re-cook).
    #[serde(default)]
    pub importer_version: u32,
    /// Hash of the source file at the time the sidecar was last updated.
    #[serde(default)]
    pub source_hash: u64,
    /// Importer settings as free-form text (for example RON).
    #[serde(default)]
    pub settings: String,
}

/// FNV-1a 64-bit hash, used for change detection (not for security).
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Whether a path is a sidecar (`.meta`) or a temporary/backup file that the registry ignores.
pub fn is_sidecar_or_temp(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    name.ends_with(".meta") || name.contains(".tmp") || name.contains(".bak")
}

/// Import state of a registered asset. Kept in memory; the `.meta` sidecar stores the hashes.
///
/// Not re-exported at the crate root (the name `AssetStatus` there is the load status of
/// [`crate::handle`]); use `redox_asset::guid::AssetStatus`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetStatus {
    /// The file is gone; the GUID is kept while references to it exist.
    Missing,
    /// An import of the asset is running.
    Importing,
    /// Imported with the current content and importer version.
    Imported,
    /// The last import failed with this message.
    Failed(String),
    /// The content changed since the last import, or the asset was never imported.
    Outdated,
}

/// A place in a scene file that refers to an asset by GUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependentRef {
    /// Asset the reference points to.
    pub guid: AssetGuid,
    /// Scene file, relative to the assets root.
    pub scene: String,
    /// GUID of the entity that holds the reference, when known.
    pub entity: Option<String>,
    /// Registered name of the component that holds the reference.
    pub component: String,
}

/// What one event (or a folder re-read) changed in the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryChange {
    /// Nothing registered changed (temporary file, sidecar, or a path outside the root).
    Ignored,
    /// A new asset was registered; its sidecar was created or read.
    Added {
        /// Path relative to the root.
        path: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// A registered asset is gone; its GUID is kept so references can still be resolved.
    Missing {
        /// Path relative to the root.
        path: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// An asset moved or was renamed together with its sidecar; the GUID is preserved.
    Moved {
        /// Old path relative to the root.
        from: String,
        /// New path relative to the root.
        to: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// The content differs from the last import, or the asset was never imported.
    Outdated {
        /// Path relative to the root.
        path: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// The content matches the last import.
    Unchanged {
        /// Path relative to the root.
        path: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// A folder changed; its files were re-read with [`AssetRegistry::resync_dir`].
    Subtree {
        /// Folder relative to the root.
        path: String,
    },
}

/// Errors of the GUID registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// A file-system operation failed.
    #[error("{path}: {source}")]
    Io {
        /// File involved.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// A sidecar could not be parsed.
    #[error("{path}: invalid .meta file: {message}")]
    BadMeta {
        /// Sidecar involved.
        path: PathBuf,
        /// Parser message.
        message: String,
    },
    /// The asset is not known to the registry.
    #[error("unknown asset {0}")]
    Unknown(String),
    /// The destination of a move already exists.
    #[error("{0} already exists")]
    Occupied(PathBuf),
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> RegistryError + '_ {
    move |source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn meta_path(asset: &Path) -> PathBuf {
    let mut name = asset.as_os_str().to_owned();
    name.push(".");
    name.push(META_EXTENSION);
    PathBuf::from(name)
}

fn write_meta(asset: &Path, meta: &AssetMeta) -> Result<(), RegistryError> {
    let path = meta_path(asset);
    let text = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::new()).map_err(|e| {
        RegistryError::BadMeta {
            path: path.clone(),
            message: e.to_string(),
        }
    })?;
    fs::write(&path, text).map_err(io_err(&path))
}

fn read_meta(asset: &Path) -> Result<Option<AssetMeta>, RegistryError> {
    let path = meta_path(asset);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(&path)(e)),
    };
    ron::from_str(&text)
        .map(Some)
        .map_err(|e| RegistryError::BadMeta {
            path,
            message: e.to_string(),
        })
}

/// What [`AssetRegistry::scan`] found and fixed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Assets that had no sidecar and received one.
    pub created: Vec<PathBuf>,
    /// Assets that shared a GUID with an earlier path and received a fresh one.
    pub regenerated: Vec<PathBuf>,
    /// Assets whose sidecar could not be read; they are left out of the registry.
    pub damaged: Vec<PathBuf>,
}

/// Maps GUIDs to asset paths (relative to the registry root, `/`-separated).
#[derive(Debug)]
pub struct AssetRegistry {
    root: PathBuf,
    by_guid: HashMap<AssetGuid, String>,
    by_path: BTreeMap<String, AssetGuid>,
    statuses: BTreeMap<String, AssetStatus>,
    scene_refs: BTreeMap<String, Vec<DependentRef>>,
}

impl AssetRegistry {
    /// Scans `root` recursively, creating missing sidecars and repairing duplicate GUIDs.
    pub fn scan(root: impl Into<PathBuf>) -> Result<(Self, ScanReport), RegistryError> {
        let root = root.into();
        let mut files = Vec::new();
        collect_files(&root, &mut files)?;
        files.sort();

        let mut registry = Self {
            root,
            by_guid: HashMap::new(),
            by_path: BTreeMap::new(),
            statuses: BTreeMap::new(),
            scene_refs: BTreeMap::new(),
        };
        let mut report = ScanReport::default();
        for file in files {
            let rel = registry.relative(&file);
            let meta = match read_meta(&file) {
                Ok(Some(meta)) => meta,
                Ok(None) => {
                    let meta = new_meta(&file)?;
                    write_meta(&file, &meta)?;
                    report.created.push(PathBuf::from(&rel));
                    meta
                }
                Err(RegistryError::BadMeta { .. }) => {
                    report.damaged.push(PathBuf::from(&rel));
                    continue;
                }
                Err(e) => return Err(e),
            };
            let mut meta = meta;
            if registry.by_guid.contains_key(&meta.guid) {
                meta.guid = registry.fresh_guid();
                write_meta(&file, &meta)?;
                report.regenerated.push(PathBuf::from(&rel));
            }
            registry.statuses.insert(rel.clone(), status_from_meta(&meta));
            registry.by_guid.insert(meta.guid, rel.clone());
            registry.by_path.insert(rel, meta.guid);
        }
        Ok((registry, report))
    }

    fn fresh_guid(&self) -> AssetGuid {
        loop {
            let guid = AssetGuid::new_random();
            if !self.by_guid.contains_key(&guid) {
                return guid;
            }
        }
    }

    fn relative(&self, file: &Path) -> String {
        file.strip_prefix(&self.root)
            .unwrap_or(file)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Root directory of the registry.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Number of registered assets.
    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    /// Whether no asset is registered.
    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }

    /// Path (relative to the root) of an asset.
    pub fn path_of(&self, guid: AssetGuid) -> Option<&str> {
        self.by_guid.get(&guid).map(String::as_str)
    }

    /// GUID of the asset at a relative path.
    pub fn guid_of(&self, path: &str) -> Option<AssetGuid> {
        self.by_path.get(path).copied()
    }

    /// Iterates over `(path, guid)` pairs in path order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, AssetGuid)> + '_ {
        self.by_path.iter().map(|(p, g)| (p.as_str(), *g))
    }

    /// Renames or moves an asset together with its sidecar; the GUID is preserved.
    pub fn move_asset(&mut self, from: &str, to: &str) -> Result<AssetGuid, RegistryError> {
        let guid = self
            .guid_of(from)
            .ok_or_else(|| RegistryError::Unknown(from.to_owned()))?;
        let src = self.root.join(from);
        let dst = self.root.join(to);
        if dst.exists() || meta_path(&dst).exists() {
            return Err(RegistryError::Occupied(dst));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        fs::rename(&src, &dst).map_err(io_err(&src))?;
        fs::rename(meta_path(&src), meta_path(&dst)).map_err(io_err(&meta_path(&src)))?;
        self.by_path.remove(from);
        self.by_path.insert(to.to_owned(), guid);
        self.by_guid.insert(guid, to.to_owned());
        if let Some(status) = self.statuses.remove(from) {
            self.statuses.insert(to.to_owned(), status);
        }
        Ok(guid)
    }

    /// Deletes an asset and its sidecar.
    pub fn remove_asset(&mut self, path: &str) -> Result<(), RegistryError> {
        let guid = self
            .guid_of(path)
            .ok_or_else(|| RegistryError::Unknown(path.to_owned()))?;
        let file = self.root.join(path);
        fs::remove_file(&file).map_err(io_err(&file))?;
        let meta = meta_path(&file);
        match fs::remove_file(&meta) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(&meta)(e)),
        }
        self.by_path.remove(path);
        self.by_guid.remove(&guid);
        self.statuses.remove(path);
        Ok(())
    }

    /// Whether the asset's file changed since its sidecar recorded the hash.
    pub fn is_modified(&self, path: &str) -> Result<bool, RegistryError> {
        let file = self.root.join(path);
        let meta = read_meta(&file)?.ok_or_else(|| RegistryError::Unknown(path.to_owned()))?;
        let bytes = fs::read(&file).map_err(io_err(&file))?;
        Ok(content_hash(&bytes) != meta.source_hash)
    }

    /// Records the current file hash in the sidecar (call after (re)importing).
    pub fn mark_imported(&self, path: &str, importer_version: u32) -> Result<(), RegistryError> {
        let file = self.root.join(path);
        let mut meta = read_meta(&file)?.ok_or_else(|| RegistryError::Unknown(path.to_owned()))?;
        let bytes = fs::read(&file).map_err(io_err(&file))?;
        meta.source_hash = content_hash(&bytes);
        meta.importer_version = importer_version;
        write_meta(&file, &meta)
    }

    /// Current status of an asset; `None` when the registry keeps none for it.
    pub fn status(&self, path: &str) -> Option<&AssetStatus> {
        self.statuses.get(path)
    }

    /// Sets the status of a registered asset; unknown paths are ignored.
    pub fn set_status(&mut self, path: &str, status: AssetStatus) {
        if self.by_path.contains_key(path) {
            self.statuses.insert(path.to_owned(), status);
        }
    }

    /// Forgets the status of an asset (used for files that have no importer).
    pub fn clear_status(&mut self, path: &str) {
        self.statuses.remove(path);
    }

    /// Replaces the asset references found in one scene file (path relative to the root).
    pub fn set_scene_references(&mut self, scene: &str, refs: Vec<DependentRef>) {
        if refs.is_empty() {
            self.scene_refs.remove(scene);
        } else {
            self.scene_refs.insert(scene.to_owned(), refs);
        }
    }

    /// Drops the references of a scene file that was deleted or is no longer indexed.
    pub fn remove_scene_references(&mut self, scene: &str) {
        self.scene_refs.remove(scene);
    }

    /// References to an asset from all indexed scenes, in scene path order.
    pub fn dependents(&self, guid: AssetGuid) -> Vec<DependentRef> {
        self.scene_refs
            .values()
            .flatten()
            .filter(|r| r.guid == guid)
            .cloned()
            .collect()
    }

    /// Applies one file-system event incrementally and reports what changed.
    pub fn apply_event(&mut self, event: &AssetEvent) -> RegistryChange {
        match event {
            AssetEvent::Created(path) => self.on_created(path),
            AssetEvent::Modified(path) => self.on_modified(path),
            AssetEvent::Removed(path) => self.on_removed(path),
            AssetEvent::Renamed { from, to } => self.on_renamed(from, to),
        }
    }

    /// Re-reads a folder: registers unknown files and marks registered files that vanished.
    pub fn resync_dir(&mut self, dir: &Path) -> Vec<RegistryChange> {
        let mut changes = Vec::new();
        let Some(rel_dir) = self.rel_of(dir) else {
            return changes;
        };
        let mut files = Vec::new();
        if dir.is_dir() && collect_files(dir, &mut files).is_err() {
            log::warn!("cannot read folder {}", dir.display());
        }
        for file in files {
            if let Some(rel) = self.rel_of(&file)
                && !self.by_path.contains_key(&rel)
            {
                changes.push(self.on_created(&file));
            }
        }
        let prefix = if rel_dir.is_empty() {
            String::new()
        } else {
            format!("{rel_dir}/")
        };
        let vanished: Vec<String> = self
            .by_path
            .keys()
            .filter(|p| p.starts_with(&prefix) && !self.root.join(p.as_str()).exists())
            .cloned()
            .collect();
        for rel in vanished {
            changes.push(self.mark_missing(&rel));
        }
        changes
    }

    fn rel_of(&self, file: &Path) -> Option<String> {
        if is_sidecar_or_temp(file) {
            return None;
        }
        let rel = file.strip_prefix(&self.root).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }

    fn on_created(&mut self, abs: &Path) -> RegistryChange {
        if abs.is_dir() {
            return self.on_subtree(abs);
        }
        let Some(rel) = self.rel_of(abs) else {
            return RegistryChange::Ignored;
        };
        if self.by_path.contains_key(&rel) {
            return self.on_modified(abs);
        }
        match self.register_file(abs, &rel) {
            Ok(change) => change,
            Err(err) => {
                log::warn!("asset {rel} was not registered: {err}");
                RegistryChange::Ignored
            }
        }
    }

    fn register_file(&mut self, abs: &Path, rel: &str) -> Result<RegistryChange, RegistryError> {
        let mut meta = match read_meta(abs)? {
            Some(meta) => meta,
            None => {
                let meta = new_meta(abs)?;
                write_meta(abs, &meta)?;
                meta
            }
        };
        if let Some(old) = self.by_guid.get(&meta.guid).cloned()
            && old != rel
        {
            if self.root.join(&old).exists() {
                // A copy of a live asset (file copied together with its sidecar).
                meta.guid = self.fresh_guid();
                write_meta(abs, &meta)?;
            } else {
                // The original is gone: this is a move seen as remove + create.
                let status = status_from_meta(&meta);
                self.move_entry(&old, rel, meta.guid, status);
                return Ok(RegistryChange::Moved {
                    from: old,
                    to: rel.to_owned(),
                    guid: meta.guid,
                });
            }
        }
        Ok(self.insert_entry(rel, &meta))
    }

    fn insert_entry(&mut self, rel: &str, meta: &AssetMeta) -> RegistryChange {
        self.statuses.insert(rel.to_owned(), status_from_meta(meta));
        self.by_guid.insert(meta.guid, rel.to_owned());
        self.by_path.insert(rel.to_owned(), meta.guid);
        RegistryChange::Added {
            path: rel.to_owned(),
            guid: meta.guid,
        }
    }

    fn move_entry(&mut self, from: &str, to: &str, guid: AssetGuid, status: AssetStatus) {
        self.by_path.remove(from);
        self.by_path.insert(to.to_owned(), guid);
        self.by_guid.insert(guid, to.to_owned());
        self.statuses.remove(from);
        self.statuses.insert(to.to_owned(), status);
    }

    fn on_modified(&mut self, abs: &Path) -> RegistryChange {
        if abs.is_dir() {
            return self.on_subtree(abs);
        }
        let Some(rel) = self.rel_of(abs) else {
            return RegistryChange::Ignored;
        };
        let Some(guid) = self.by_path.get(&rel).copied() else {
            return self.on_created(abs);
        };
        let bytes = match fs::read(abs) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return self.on_removed(abs),
            Err(err) => {
                log::warn!("asset {rel} cannot be read: {err}");
                return RegistryChange::Ignored;
            }
        };
        match read_meta(abs).ok().flatten() {
            Some(meta) if meta.source_hash == content_hash(&bytes) => {
                if self.statuses.get(&rel) == Some(&AssetStatus::Missing) {
                    self.statuses.insert(rel.clone(), status_from_meta(&meta));
                }
                RegistryChange::Unchanged { path: rel, guid }
            }
            _ => {
                self.statuses.insert(rel.clone(), AssetStatus::Outdated);
                RegistryChange::Outdated { path: rel, guid }
            }
        }
    }

    fn on_removed(&mut self, abs: &Path) -> RegistryChange {
        let Some(rel) = self.rel_of(abs) else {
            return RegistryChange::Ignored;
        };
        if self.by_path.contains_key(&rel) {
            return self.mark_missing(&rel);
        }
        let prefix = format!("{rel}/");
        if self.by_path.keys().any(|p| p.starts_with(&prefix)) {
            self.resync_dir(abs);
            return RegistryChange::Subtree { path: rel };
        }
        RegistryChange::Ignored
    }

    fn on_renamed(&mut self, from: &Path, to: &Path) -> RegistryChange {
        match (self.rel_of(from), self.rel_of(to)) {
            (Some(_), Some(to_rel)) if to.is_dir() => {
                // Folder: the destination first, so moved sidecars pair with their old entries.
                self.resync_dir(to);
                self.resync_dir(from);
                RegistryChange::Subtree { path: to_rel }
            }
            (Some(_), Some(_)) => {
                self.on_removed(from);
                self.on_created(to)
            }
            (Some(_), None) => self.on_removed(from),
            (None, Some(_)) => self.on_created(to),
            (None, None) => RegistryChange::Ignored,
        }
    }

    fn on_subtree(&mut self, dir: &Path) -> RegistryChange {
        let Some(rel) = self.rel_of(dir) else {
            return RegistryChange::Ignored;
        };
        self.resync_dir(dir);
        RegistryChange::Subtree { path: rel }
    }

    fn mark_missing(&mut self, rel: &str) -> RegistryChange {
        let Some(guid) = self.by_path.get(rel).copied() else {
            return RegistryChange::Ignored;
        };
        self.statuses.insert(rel.to_owned(), AssetStatus::Missing);
        RegistryChange::Missing {
            path: rel.to_owned(),
            guid,
        }
    }
}

fn status_from_meta(meta: &AssetMeta) -> AssetStatus {
    if meta.importer_version == 0 {
        AssetStatus::Outdated
    } else {
        AssetStatus::Imported
    }
}

fn new_meta(file: &Path) -> Result<AssetMeta, RegistryError> {
    let bytes = fs::read(file).map_err(io_err(file))?;
    Ok(AssetMeta {
        guid: AssetGuid::new_random(),
        importer_version: 0,
        source_hash: content_hash(&bytes),
        settings: String::new(),
    })
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), RegistryError> {
    for entry in fs::read_dir(dir).map_err(io_err(dir))? {
        let path = entry.map_err(io_err(dir))?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.extension().is_none_or(|e| e != META_EXTENSION) {
            out.push(path);
        }
    }
    Ok(())
}

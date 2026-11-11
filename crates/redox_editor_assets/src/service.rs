//! Asset service of the editor. Owns the GUID registry, the file watcher, the reference index and
//! the import queue. Applies file-system changes and import results incrementally in
//! [`AssetService::tick`], and performs moves and deletes that keep references intact.

use std::fs;
use std::path::{Path, PathBuf};

use redox_asset::guid::AssetStatus;
use redox_asset::{
    AssetEvent, AssetGuid, AssetRegistry, DependentRef, HotReloadWatcher, RegistryChange,
    RegistryError, ScanReport,
};

use crate::import::ImporterRegistry;
use crate::import_queue::{ImportEvent, ImportQueue, default_worker_count};
use crate::reference_index::{
    MissingRef, RefEdit, RefResolution, ReferenceError, ReferenceIndex, edit_scene_refs,
    is_scene_path, resolve_ref,
};

/// Errors of the asset service.
#[derive(Debug, thiserror::Error)]
pub enum AssetServiceError {
    /// A registry operation failed.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// A scene file could not be read or written.
    #[error(transparent)]
    Reference(#[from] ReferenceError),
    /// The asset is not registered.
    #[error("asset {0} is not registered")]
    NotFound(String),
    /// A folder could not be created or a sidecar could not be removed.
    #[error("{path}: {source}")]
    Io {
        /// Path involved.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
}

/// Import settings of the service. The editor settings (EDT-A4) fill it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportSettings {
    /// Number of import worker threads.
    pub workers: usize,
    /// Whether outdated assets are imported automatically.
    pub auto_import: bool,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            workers: default_worker_count(),
            auto_import: true,
        }
    }
}

/// Import counts for the status bar ("Importing N / Failed M").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportCounts {
    /// Jobs queued or running.
    pub importing: usize,
    /// Assets whose last import failed.
    pub failed: usize,
}

/// Diagnostics produced when the assets root is opened (or rescanned).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetReport {
    /// Assets that had no sidecar and received one.
    pub created: Vec<PathBuf>,
    /// Assets that shared a GUID with another asset (a copy) and received a fresh one.
    pub regenerated: Vec<PathBuf>,
    /// Assets whose sidecar is damaged; they are left out of the registry until repaired.
    pub damaged: Vec<PathBuf>,
    /// Asset references in scenes that point to assets that do not exist.
    pub missing_refs: Vec<MissingRef>,
    /// Scene files that could not be read, with the reason.
    pub unreadable_scenes: Vec<String>,
}

/// A registry or import change the editor should react to (browser, log, dirty flags).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetChange {
    /// A new asset was registered.
    Added {
        /// Path relative to the root.
        path: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// A registered asset disappeared; references to it are now missing.
    Missing {
        /// Path relative to the root.
        path: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// An asset moved (outside the editor or through it).
    Moved {
        /// Old path relative to the root.
        from: String,
        /// New path relative to the root.
        to: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// An asset changed on disk and needs an import.
    Outdated {
        /// Path relative to the root.
        path: String,
        /// Identity of the asset.
        guid: AssetGuid,
    },
    /// An import finished successfully.
    Imported {
        /// Path relative to the root.
        path: String,
    },
    /// An import failed; the message explains why.
    ImportFailed {
        /// Path relative to the root.
        path: String,
        /// Explanation for the user.
        message: String,
    },
}

/// Result of a move performed through the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveReport {
    /// Identity of the moved asset (unchanged by the move).
    pub guid: AssetGuid,
    /// Old path relative to the root.
    pub from: String,
    /// New path relative to the root.
    pub to: String,
    /// Scene files whose path hints for this asset are now stale (sorted, unique).
    pub scenes: Vec<String>,
}

/// Result of a delete request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    /// The asset is still referenced and `force` was not given; nothing was deleted.
    Blocked {
        /// Scenes and entities that reference the asset.
        dependents: Vec<DependentRef>,
    },
    /// The asset was deleted. The references in `dangling` now resolve to missing assets.
    Deleted {
        /// Identity of the deleted asset.
        guid: AssetGuid,
        /// References that became missing.
        dangling: Vec<DependentRef>,
    },
}

/// Fix applied to one missing reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefFix {
    /// Point the reference at another registered asset.
    Reassign(AssetGuid),
    /// Remove the reference (GUID and path become null).
    Clear,
}

/// A move of an asset as an undoable operation. The coordinator wires it to the undo stack
/// (`UndoCommand::MoveAsset` in `redox_editor_core`); [`Self::revert`] is the undo action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetMoveOp {
    /// Path before the move, relative to the root.
    pub from: String,
    /// Path after the move, relative to the root.
    pub to: String,
}

impl AssetMoveOp {
    /// Moves the asset from `from` to `to`.
    pub fn apply(&self, service: &mut AssetService) -> Result<MoveReport, AssetServiceError> {
        service.move_asset(&self.from, &self.to)
    }

    /// Moves the asset back from `to` to `from`.
    pub fn revert(&self, service: &mut AssetService) -> Result<MoveReport, AssetServiceError> {
        service.move_asset(&self.to, &self.from)
    }
}

/// The editor's asset service.
pub struct AssetService {
    registry: AssetRegistry,
    watcher: Option<HotReloadWatcher>,
    references: ReferenceIndex,
    report: AssetReport,
    importers: ImporterRegistry,
    queue: ImportQueue,
    auto_import: bool,
}

impl AssetService {
    /// Opens the assets root with default import settings.
    pub fn open(assets_root: impl Into<PathBuf>) -> Result<Self, AssetServiceError> {
        Self::open_with(assets_root, ImportSettings::default())
    }

    /// Opens the assets root: scans it, indexes scenes, starts watching it, builds the report,
    /// and queues imports for outdated assets.
    pub fn open_with(
        assets_root: impl Into<PathBuf>,
        settings: ImportSettings,
    ) -> Result<Self, AssetServiceError> {
        let root = assets_root.into();
        fs::create_dir_all(&root).map_err(|source| AssetServiceError::Io {
            path: root.clone(),
            source,
        })?;
        let (mut registry, scan) = AssetRegistry::scan(root.clone())?;
        let mut references = ReferenceIndex::default();
        let unreadable = references.rebuild(&mut registry);
        let importers = ImporterRegistry::with_builtins();
        let mut service = Self {
            registry,
            watcher: start_watcher(&root),
            references,
            report: AssetReport::default(),
            queue: ImportQueue::new(importers.clone(), settings.workers),
            importers,
            auto_import: settings.auto_import,
        };
        service.settle_statuses();
        service.report = service.build_report(scan, unreadable);
        service.queue_outdated();
        Ok(service)
    }

    /// Rescans the root from scratch (Rescan/Repair command) and returns the new report.
    pub fn rescan(&mut self) -> Result<AssetReport, AssetServiceError> {
        let root = self.registry.root().to_path_buf();
        let (mut registry, scan) = AssetRegistry::scan(root)?;
        let mut references = ReferenceIndex::default();
        let unreadable = references.rebuild(&mut registry);
        self.registry = registry;
        self.references = references;
        self.settle_statuses();
        self.report = self.build_report(scan, unreadable);
        self.queue_outdated();
        Ok(self.report.clone())
    }

    /// Applies pending file-system events and import results; called by the main loop.
    pub fn tick(&mut self) -> Vec<AssetChange> {
        let events = self
            .watcher
            .as_ref()
            .map(HotReloadWatcher::drain_events)
            .unwrap_or_default();
        let mut changes = Vec::new();
        let mut rebuild_all = false;
        for event in &events {
            match self.registry.apply_event(event) {
                RegistryChange::Subtree { .. } => {
                    rebuild_all = true;
                    for change in self.registry.take_folder_changes() {
                        self.absorb(change, &mut changes);
                    }
                }
                change => self.absorb(change, &mut changes),
            }
        }
        if rebuild_all {
            for err in self.references.rebuild(&mut self.registry) {
                log::warn!("scene not indexed: {err}");
            }
            self.settle_statuses();
        }
        let import_events = self.queue.drain_events();
        let imported_any = !import_events.is_empty();
        for event in import_events {
            self.apply_import_event(event, &mut changes);
        }
        if self.auto_import && (!events.is_empty() || imported_any || rebuild_all) {
            self.queue_outdated();
        }
        changes
    }

    /// Registry view of the root.
    pub fn registry(&self) -> &AssetRegistry {
        &self.registry
    }

    /// Diagnostics of the last open or rescan.
    pub fn report(&self) -> &AssetReport {
        &self.report
    }

    /// Current path of an asset.
    pub fn resolve(&self, guid: AssetGuid) -> Option<&str> {
        self.registry.path_of(guid)
    }

    /// GUID of the asset at a path.
    pub fn guid_of(&self, path: &str) -> Option<AssetGuid> {
        self.registry.guid_of(path)
    }

    /// Import status of an asset.
    pub fn status(&self, path: &str) -> Option<&AssetStatus> {
        self.registry.status(path)
    }

    /// Scene references to an asset.
    pub fn dependents(&self, guid: AssetGuid) -> Vec<DependentRef> {
        self.registry.dependents(guid)
    }

    /// References in scenes whose asset cannot be resolved right now.
    pub fn missing_refs(&self) -> Vec<MissingRef> {
        self.references.missing_refs(&self.registry)
    }

    /// Current path of the asset whose GUID is written as 32 hex digits.
    pub fn path_of_text(&self, guid_text: &str) -> Option<String> {
        let parsed = u128::from_str_radix(guid_text, 16).ok()?;
        self.resolve(AssetGuid(parsed)).map(str::to_owned)
    }

    /// Resolves one reference (GUID text and path hint) against the registry.
    pub fn resolve_ref(&self, guid_text: Option<&str>, path_hint: Option<&str>) -> RefResolution {
        resolve_ref(&self.registry, guid_text, path_hint)
    }

    /// Import counts for the status bar.
    pub fn import_counts(&self) -> ImportCounts {
        let failed = self
            .registry
            .iter()
            .filter(|(path, _)| matches!(self.registry.status(path), Some(AssetStatus::Failed(_))))
            .count();
        ImportCounts {
            importing: self.queue.pending_count() + self.queue.active_count(),
            failed,
        }
    }

    /// Turns automatic import of outdated assets on or off.
    pub fn set_auto_import(&mut self, enabled: bool) {
        self.auto_import = enabled;
    }

    /// Sets the asset open in the editor (its queued imports run first); `None` clears it.
    pub fn set_open_asset(&mut self, path: Option<&str>) {
        let root = self.registry.root().to_path_buf();
        self.queue.set_open(path.map(|p| root.join(p)));
    }

    /// Queues an import of a registered asset. Returns `false` when it is already queued, has no
    /// importer, or is not registered.
    pub fn import(&mut self, path: &str) -> bool {
        match self.registry.guid_of(path) {
            Some(guid) => self.queue.submit(self.registry.root().join(path), guid),
            None => false,
        }
    }

    /// Queues again an asset whose last import failed or was cancelled.
    pub fn retry_import(&mut self, path: &str) -> bool {
        let abs = self.registry.root().join(path);
        self.queue.retry(&abs)
    }

    /// Cancels a queued or running import of an asset.
    pub fn cancel_import(&mut self, path: &str) -> bool {
        let abs = self.registry.root().join(path);
        self.queue.cancel(&abs)
    }

    /// Queues an import for every outdated asset (never imported, or changed since). Returns the
    /// number of jobs added.
    pub fn queue_outdated(&mut self) -> usize {
        if !self.auto_import {
            return 0;
        }
        let root = self.registry.root().to_path_buf();
        let outdated: Vec<(PathBuf, AssetGuid)> = self
            .registry
            .iter()
            .filter(|(path, _)| self.registry.status(path) == Some(&AssetStatus::Outdated))
            .map(|(path, guid)| (root.join(path), guid))
            .collect();
        outdated
            .into_iter()
            .filter(|(path, guid)| self.queue.submit(path.clone(), *guid))
            .count()
    }

    /// Moves or renames an asset with its sidecar. The GUID is preserved, so references keep
    /// resolving. Path hints in scenes are not rewritten here: see [`Self::update_path_hints`],
    /// which the caller runs after the user confirms it (the report lists the scenes).
    pub fn move_asset(&mut self, from: &str, to: &str) -> Result<MoveReport, AssetServiceError> {
        let guid = self.registry.move_asset(from, to)?;
        if is_scene_path(from) {
            self.references.remove_scene(&mut self.registry, from);
        }
        if is_scene_path(to) {
            self.references.update_scene(&mut self.registry, to)?;
        }
        Ok(MoveReport {
            guid,
            from: from.to_owned(),
            to: to.to_owned(),
            scenes: self.scenes_of(guid),
        })
    }

    /// Rewrites the path hints of every closed scene that references the asset. The GUID does
    /// not change. Returns the number of rewritten references.
    pub fn update_path_hints(&mut self, guid: AssetGuid) -> Result<usize, AssetServiceError> {
        let new_path = self
            .registry
            .path_of(guid)
            .map(str::to_owned)
            .ok_or_else(|| AssetServiceError::NotFound(guid.to_string()))?;
        let guid_text = guid.to_string();
        let root = self.registry.root().to_path_buf();
        let mut total = 0;
        for scene in self.scenes_of(guid) {
            total += edit_scene_refs(&root, &scene, |sref| {
                if sref.guid.as_deref() == Some(guid_text.as_str()) {
                    RefEdit::SetPath(new_path.clone())
                } else {
                    RefEdit::Keep
                }
            })?;
            self.references.update_scene(&mut self.registry, &scene)?;
        }
        Ok(total)
    }

    /// Deletes an asset. Without `force` a referenced asset is not deleted; the caller shows the
    /// dependents in a confirmation dialog and repeats the call with `force`.
    pub fn delete_asset(
        &mut self,
        path: &str,
        force: bool,
    ) -> Result<DeleteOutcome, AssetServiceError> {
        let guid = self
            .registry
            .guid_of(path)
            .ok_or_else(|| AssetServiceError::NotFound(path.to_owned()))?;
        let dependents = self.registry.dependents(guid);
        if !dependents.is_empty() && !force {
            return Ok(DeleteOutcome::Blocked { dependents });
        }
        self.registry.remove_asset(path)?;
        if is_scene_path(path) {
            self.references.remove_scene(&mut self.registry, path);
        }
        Ok(DeleteOutcome::Deleted {
            guid,
            dangling: dependents,
        })
    }

    /// Applies a fix to one missing reference: reassigns it or clears it.
    /// Returns the number of rewritten references in that scene.
    pub fn fix_missing_ref(
        &mut self,
        target: &MissingRef,
        fix: &RefFix,
    ) -> Result<usize, AssetServiceError> {
        let replacement = match fix {
            RefFix::Reassign(guid) => Some((
                guid.to_string(),
                self.registry.path_of(*guid).map(str::to_owned),
            )),
            RefFix::Clear => None,
        };
        let root = self.registry.root().to_path_buf();
        let changed = edit_scene_refs(&root, &target.scene, |sref| {
            let same = sref.entity == target.entity
                && sref.component == target.component
                && sref.guid == target.guid
                && sref.path == target.path;
            match (same, &replacement) {
                (false, _) => RefEdit::Keep,
                (true, Some((guid, path))) => RefEdit::Reassign {
                    guid: guid.clone(),
                    path: path.clone(),
                },
                (true, None) => RefEdit::Clear,
            }
        })?;
        self.references
            .update_scene(&mut self.registry, &target.scene)?;
        Ok(changed)
    }

    /// Repairs a damaged sidecar: removes it and registers the file again with a new sidecar.
    pub fn repair_sidecar(&mut self, path: &str) -> Result<Vec<AssetChange>, AssetServiceError> {
        let abs = self.registry.root().join(path);
        let mut sidecar = abs.as_os_str().to_owned();
        sidecar.push(".meta");
        match fs::remove_file(PathBuf::from(sidecar)) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(AssetServiceError::Io { path: abs, source });
            }
        }
        let change = self.registry.apply_event(&AssetEvent::Created(abs));
        let mut out = Vec::new();
        self.absorb(change, &mut out);
        Ok(out)
    }

    fn scenes_of(&self, guid: AssetGuid) -> Vec<String> {
        let mut scenes: Vec<String> = self
            .registry
            .dependents(guid)
            .into_iter()
            .map(|r| r.scene)
            .collect();
        scenes.sort();
        scenes.dedup();
        scenes
    }

    fn absorb(&mut self, change: RegistryChange, out: &mut Vec<AssetChange>) {
        match change {
            RegistryChange::Ignored
            | RegistryChange::Unchanged { .. }
            | RegistryChange::Subtree { .. } => {}
            RegistryChange::Added { path, guid } => {
                self.settle(&path);
                self.reindex_if_scene(&path);
                out.push(AssetChange::Added { path, guid });
            }
            RegistryChange::Missing { path, guid } => {
                if is_scene_path(&path) {
                    self.references.remove_scene(&mut self.registry, &path);
                }
                out.push(AssetChange::Missing { path, guid });
            }
            RegistryChange::Moved { from, to, guid } => {
                if is_scene_path(&from) {
                    self.references.remove_scene(&mut self.registry, &from);
                }
                self.settle(&to);
                self.reindex_if_scene(&to);
                out.push(AssetChange::Moved { from, to, guid });
            }
            RegistryChange::Outdated { path, guid } => {
                self.settle(&path);
                self.reindex_if_scene(&path);
                out.push(AssetChange::Outdated { path, guid });
            }
        }
    }

    fn apply_import_event(&mut self, event: ImportEvent, out: &mut Vec<AssetChange>) {
        match event {
            ImportEvent::Started { path, .. } => {
                if let Some(rel) = self.relative(&path) {
                    self.registry.set_status(&rel, AssetStatus::Importing);
                }
            }
            ImportEvent::Progress { .. } => {}
            ImportEvent::Finished {
                path,
                importer_version,
                report,
                ..
            } => {
                let Some(rel) = self.relative(&path) else {
                    return;
                };
                for info in &report.info {
                    log::info!("import {rel}: {info}");
                }
                for warning in &report.warnings {
                    log::warn!("import {rel}: {warning}");
                }
                match self.registry.mark_imported(&rel, importer_version) {
                    Ok(()) => {
                        self.registry.set_status(&rel, AssetStatus::Imported);
                        out.push(AssetChange::Imported { path: rel });
                    }
                    Err(err) => {
                        let message = err.to_string();
                        self.registry
                            .set_status(&rel, AssetStatus::Failed(message.clone()));
                        out.push(AssetChange::ImportFailed { path: rel, message });
                    }
                }
            }
            ImportEvent::Failed { path, message, .. } => {
                if let Some(rel) = self.relative(&path) {
                    self.registry
                        .set_status(&rel, AssetStatus::Failed(message.clone()));
                    out.push(AssetChange::ImportFailed { path: rel, message });
                }
            }
            ImportEvent::Cancelled { path, .. } => {
                // Cancelled imports wait for an explicit retry, so that auto-import does not
                // start them again.
                if let Some(rel) = self.relative(&path) {
                    self.registry
                        .set_status(&rel, AssetStatus::Failed("import cancelled".to_owned()));
                }
            }
        }
    }

    /// Clears the status of a file that has no importer (it never needs an import).
    /// A `Missing` status is kept, because references must see that the file is gone.
    fn settle(&mut self, path: &str) {
        if self.importers.for_path(Path::new(path)).is_none()
            && self.registry.status(path) != Some(&AssetStatus::Missing)
        {
            self.registry.clear_status(path);
        }
    }

    fn settle_statuses(&mut self) {
        let paths: Vec<String> = self
            .registry
            .iter()
            .map(|(path, _)| path.to_owned())
            .collect();
        for path in paths {
            self.settle(&path);
        }
    }

    fn relative(&self, abs: &Path) -> Option<String> {
        let rel = abs.strip_prefix(self.registry.root()).ok()?;
        Some(
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
        )
    }

    fn reindex_if_scene(&mut self, path: &str) {
        if is_scene_path(path)
            && let Err(err) = self.references.update_scene(&mut self.registry, path)
        {
            log::warn!("scene {path} cannot be indexed: {err}");
        }
    }

    fn build_report(&self, scan: ScanReport, unreadable: Vec<ReferenceError>) -> AssetReport {
        AssetReport {
            created: scan.created,
            regenerated: scan.regenerated,
            damaged: scan.damaged,
            missing_refs: self.missing_refs(),
            unreadable_scenes: unreadable.iter().map(ToString::to_string).collect(),
        }
    }
}

fn start_watcher(root: &Path) -> Option<HotReloadWatcher> {
    let mut watcher = match HotReloadWatcher::new() {
        Ok(watcher) => watcher,
        Err(err) => {
            log::warn!("file watcher unavailable: {err}");
            return None;
        }
    };
    if let Err(err) = watcher.watch(root) {
        log::warn!("cannot watch {}: {err}", root.display());
        return None;
    }
    Some(watcher)
}

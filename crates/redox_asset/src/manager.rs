//! Central asset manager — ECS resource for loading and accessing assets.
//!
//! Built-in loaders (`ImageLoader`, `GltfLoader`) are registered by default;
//! use [`register_loader`](AssetManager::register_loader) to add or override
//! loaders, then use [`load_async`](AssetManager::load_async) to load by path.
//! Call [`update`](AssetManager::update) each frame to process completed loads
//! and emit ECS events.
//!
//! # Lifetime
//!
//! Handles stay `Copy`; ownership is explicit. Every `load_async`/`insert` call counts as one
//! owner, [`retain`](AssetManager::retain) adds one and [`release`](AssetManager::release)
//! drops one. At zero the asset is unloaded together with the dependencies registered with
//! [`add_dependency`](AssetManager::add_dependency) (each of which loses one owner).
//! With [`set_source`](AssetManager::set_source) bytes come from a folder or `.pak` instead of
//! the file system, and [`set_completion_budget`](AssetManager::set_completion_budget) spreads
//! finished loads over frames so streaming does not stall a frame.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use flume::{Receiver, Sender};
use redox_ecs::World;

use crate::error::AssetError;
use crate::event::{AssetLoadFailedEvent, AssetLoadedEvent, AssetReloadedEvent};
use crate::handle::{AssetId, AssetStatus, Handle};
use crate::hot_reload::HotReloadWatcher;
use crate::loader::{AssetLoader, GltfLoader, GltfSceneData, ImageLoader};
use crate::pak::AssetSource;
use crate::storage::AssetStorage;

/// Task sent to the loader thread.
struct LoadTask {
    type_id: TypeId,
    id: AssetId,
    path: PathBuf,
    is_reload: bool,
    /// Alternative byte source and the path inside it (`None`: read the file system).
    source: Option<(Arc<dyn AssetSource>, String)>,
}

/// Result sent back from the loader thread.
struct LoadComplete {
    type_id: TypeId,
    id: AssetId,
    path: PathBuf,
    is_reload: bool,
    result: Result<Box<dyn Any + Send + Sync>, String>,
}

/// Type-erased loader for the registry.
trait LoaderErased: Send + Sync + 'static {
    fn load_from_path(
        &self,
        path: &Path,
        bytes: &[u8],
    ) -> Result<Box<dyn Any + Send + Sync>, crate::error::AssetError>;
    #[allow(dead_code)]
    fn extensions(&self) -> Vec<&'static str>;
}

struct LoaderWrapper<L: AssetLoader> {
    inner: L,
}

impl<L: AssetLoader> LoaderErased for LoaderWrapper<L> {
    fn load_from_path(
        &self,
        path: &Path,
        bytes: &[u8],
    ) -> Result<Box<dyn Any + Send + Sync>, crate::error::AssetError> {
        self.inner
            .load_from_path(path, bytes)
            .map(|a| Box::new(a) as Box<dyn Any + Send + Sync>)
    }
    fn extensions(&self) -> Vec<&'static str> {
        self.inner.extensions()
    }
}

type LoaderRegistry = Arc<RwLock<HashMap<TypeId, Box<dyn LoaderErased>>>>;

fn worker_loop(registry: LoaderRegistry, rx: Receiver<LoadTask>, tx: Sender<LoadComplete>) {
    while let Ok(task) = rx.recv() {
        let read = match &task.source {
            Some((source, rel)) => source.read(rel),
            None => std::fs::File::open(&task.path).and_then(|mut f| {
                let mut v = Vec::new();
                f.read_to_end(&mut v).map(|_| v)
            }),
        };
        let bytes = match read {
            Ok(b) => b,
            Err(e) => {
                let _ = tx.send(LoadComplete {
                    type_id: task.type_id,
                    id: task.id,
                    path: task.path,
                    is_reload: task.is_reload,
                    result: Err(format!("{:?}", e)),
                });
                continue;
            }
        };

        let result = {
            let guard = match registry.read() {
                Ok(g) => g,
                Err(_) => {
                    let _ = tx.send(LoadComplete {
                        type_id: task.type_id,
                        id: task.id,
                        path: task.path.clone(),
                        is_reload: task.is_reload,
                        result: Err("registry lock failed".to_string()),
                    });
                    continue;
                }
            };
            match guard.get(&task.type_id) {
                Some(loader) => loader
                    .load_from_path(&task.path, &bytes)
                    .map_err(|e| e.to_string()),
                None => {
                    drop(guard);
                    let _ = tx.send(LoadComplete {
                        type_id: task.type_id,
                        id: task.id,
                        path: task.path.clone(),
                        is_reload: task.is_reload,
                        result: Err("no loader for type".to_string()),
                    });
                    continue;
                }
            }
        };
        let _ = tx.send(LoadComplete {
            type_id: task.type_id,
            id: task.id,
            path: task.path,
            is_reload: task.is_reload,
            result,
        });
    }
}

/// Central asset manager.
///
/// Insert as an ECS resource. Register loaders, then use `load_async` to load by path.
/// Call `update(world)` each frame to process completions and emit events.
pub struct AssetManager {
    /// Base path for asset resolution.
    pub base_path: PathBuf,
    /// Per-type storages.
    storages: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    /// Insert loaded asset into storage (type_id -> inserter).
    inserters: HashMap<TypeId, fn(&mut dyn Any, AssetId, Box<dyn Any + Send + Sync>)>,
    /// Mark asset as failed (type_id -> marker).
    failed_markers: HashMap<TypeId, fn(&mut dyn Any, AssetId)>,
    /// Path -> (type_id, asset_id) for cache and hot-reload.
    path_to_handle: HashMap<PathBuf, (TypeId, AssetId)>,
    /// Loader registry (shared with worker thread).
    loaders: LoaderRegistry,
    task_tx: Sender<LoadTask>,
    complete_tx: Sender<LoadComplete>,
    complete_rx: Receiver<LoadComplete>,
    /// Optional file watcher for hot-reload. When set, [`update`](Self::update) drains
    /// change events and re-queues loads for affected paths.
    pub watcher: Option<HotReloadWatcher>,
    /// Owners per live asset id.
    refs: HashMap<AssetId, u32>,
    /// Asset type per live id (needed to unload by id).
    kinds: HashMap<AssetId, TypeId>,
    /// parent -> assets it keeps alive.
    deps: HashMap<AssetId, Vec<AssetId>>,
    /// Type-erased storage removal.
    removers: HashMap<TypeId, fn(&mut dyn Any, AssetId)>,
    /// Type-erased status lookup.
    statusers: HashMap<TypeId, fn(&dyn Any, AssetId) -> AssetStatus>,
    /// Where load bytes come from; `None` reads `base_path` on disk.
    source: Option<Arc<dyn AssetSource>>,
    /// Max finished loads applied per [`update`](Self::update).
    completion_budget: usize,
}

impl AssetManager {
    /// Creates a new asset manager with the given base path.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        let (task_tx, task_rx) = flume::unbounded();
        let (complete_tx, complete_rx) = flume::unbounded();
        // Pre-register the built-in loaders so common formats load out of
        // the box; callers may replace any slot via `register_loader`.
        let loaders: LoaderRegistry = Arc::new(RwLock::new(HashMap::new()));
        let registry_clone = Arc::clone(&loaders);
        let complete_tx_worker = complete_tx.clone();
        std::thread::spawn(move || worker_loop(registry_clone, task_rx, complete_tx_worker));

        let mut manager = Self {
            base_path: base_path.into(),
            storages: HashMap::new(),
            inserters: HashMap::new(),
            failed_markers: HashMap::new(),
            path_to_handle: HashMap::new(),
            loaders,
            task_tx,
            complete_tx,
            complete_rx,
            watcher: None,
            refs: HashMap::new(),
            kinds: HashMap::new(),
            deps: HashMap::new(),
            removers: HashMap::new(),
            statusers: HashMap::new(),
            source: None,
            completion_budget: usize::MAX,
        };
        // Built-in formats; callers may replace any of them with `register_loader`.
        manager.register_loader::<image::DynamicImage, _>(ImageLoader);
        manager.register_loader::<GltfSceneData, _>(GltfLoader);
        manager.register_loader::<crate::loader::Ktx2Texture, _>(crate::loader::Ktx2Loader);
        manager
    }

    /// Enables hot-reload by creating a file watcher. Call [`watch`](HotReloadWatcher::watch)
    /// on `self.watcher` to add paths to watch.
    pub fn enable_hot_reload(&mut self) -> notify::Result<()> {
        self.watcher = Some(HotReloadWatcher::new()?);
        Ok(())
    }

    /// Registers a loader for asset type `T`. Required before calling `load_async::<T>`.
    pub fn register_loader<T, L>(&mut self, loader: L)
    where
        T: 'static + Send + Sync,
        L: AssetLoader<Asset = T> + 'static,
    {
        let type_id = TypeId::of::<T>();
        if let Ok(mut r) = self.loaders.write() {
            r.insert(type_id, Box::new(LoaderWrapper { inner: loader }));
        }

        self.inserters.entry(type_id).or_insert(|any, id, boxed| {
            if let Some(s) = any.downcast_mut::<AssetStorage<T>>()
                && let Ok(asset) = boxed.downcast::<T>()
            {
                s.insert(Handle::new(id), *asset);
            }
        });

        self.failed_markers.entry(type_id).or_insert(|any, id| {
            if let Some(s) = any.downcast_mut::<AssetStorage<T>>() {
                s.mark_failed(Handle::new(id));
            }
        });

        self.storages
            .entry(type_id)
            .or_insert_with(|| Box::new(AssetStorage::<T>::new()));
        self.register_type_ops::<T>();
    }

    fn register_type_ops<T: 'static + Send + Sync>(&mut self) {
        let type_id = TypeId::of::<T>();
        self.removers.entry(type_id).or_insert(|any, id| {
            if let Some(s) = any.downcast_mut::<AssetStorage<T>>() {
                s.remove(Handle::new(id));
            }
        });
        self.statusers.entry(type_id).or_insert(|any, id| {
            any.downcast_ref::<AssetStorage<T>>()
                .map_or(AssetStatus::Failed, |s| s.status(Handle::new(id)))
        });
    }

    /// Reads load bytes from `source` (a folder or `.pak`) instead of the file system.
    /// Paths given to `load_async` are then relative to the source root.
    pub fn set_source(&mut self, source: Arc<dyn AssetSource>) {
        self.source = Some(source);
    }

    /// Caps how many finished loads one [`update`](Self::update) applies, so heavy assets
    /// stream in over several frames. Default: unlimited.
    pub fn set_completion_budget(&mut self, per_frame: usize) {
        self.completion_budget = per_frame.max(1);
    }

    /// Returns the storage for type `T`, if it exists.
    pub fn storage<T: 'static + Send + Sync>(&self) -> Option<&AssetStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<AssetStorage<T>>())
    }

    fn storage_mut<T: 'static + Send + Sync>(&mut self) -> &mut AssetStorage<T> {
        let type_id = TypeId::of::<T>();
        self.storages
            .entry(type_id)
            .or_insert_with(|| Box::new(AssetStorage::<T>::new()));
        self.register_type_ops::<T>();
        self.storages
            .get_mut(&type_id)
            .and_then(|s| s.downcast_mut::<AssetStorage<T>>())
            .expect("type mismatch")
    }

    /// Inserts an already-loaded asset and returns a handle.
    pub fn insert<T: 'static + Send + Sync>(&mut self, asset: T) -> Handle<T> {
        let handle = Handle::new(AssetId::next());
        self.storage_mut::<T>().insert(handle, asset);
        self.track(handle.id, TypeId::of::<T>());
        handle
    }

    /// Returns a reference to a loaded asset.
    pub fn get<T: 'static + Send + Sync>(&self, handle: Handle<T>) -> Option<&T> {
        self.storage::<T>()?.get(handle)
    }

    /// Returns a mutable reference to a loaded asset.
    pub fn get_mut<T: 'static + Send + Sync>(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.storage_mut::<T>().get_mut(handle)
    }

    /// Returns the status of an asset.
    pub fn status<T: 'static + Send + Sync>(&self, handle: Handle<T>) -> AssetStatus {
        self.storage::<T>()
            .map(|s| s.status(handle))
            .unwrap_or(AssetStatus::Failed)
    }

    /// Resolves a relative path against the base path.
    pub fn resolve_path(&self, relative: &str) -> PathBuf {
        self.base_path.join(relative)
    }

    /// Starts an asynchronous load by path. Returns a handle immediately (status `Loading` until done).
    ///
    /// If this path was already loaded or is loading, returns the existing handle (cached).
    /// Requires a loader for `T` to be registered.
    pub fn load_async<T: 'static + Send + Sync>(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Handle<T>, AssetError> {
        let relative = path.as_ref().to_path_buf();
        let path = self.base_path.join(&relative);

        let type_id = TypeId::of::<T>();
        if let Some((tid, id)) = self.path_to_handle.get(&path)
            && *tid == type_id
        {
            let id = *id;
            *self.refs.entry(id).or_insert(0) += 1;
            return Ok(Handle::new(id));
        }

        if !self
            .loaders
            .read()
            .map(|r| r.contains_key(&type_id))
            .unwrap_or(false)
        {
            return Err(AssetError::UnsupportedFormat(format!(
                "no loader registered for type_id {:?}",
                type_id
            )));
        }

        let id = AssetId::next();
        self.storage_mut::<T>().mark_loading(Handle::new(id));
        self.path_to_handle.insert(path.clone(), (type_id, id));
        self.track(id, type_id);

        let source = self.source.clone().map(|s| {
            let rel = relative.to_string_lossy().replace('\\', "/");
            (s, rel)
        });
        let _ = self.task_tx.send(LoadTask {
            type_id,
            id,
            path,
            is_reload: false,
            source,
        });

        Ok(Handle::new(id))
    }

    /// Queues a reload for an already-known path (e.g. from hot-reload). Reuses the same handle.
    pub fn reload_async(&mut self, path: PathBuf, type_id: TypeId, id: AssetId) {
        let _ = self.task_tx.send(LoadTask {
            type_id,
            id,
            path,
            is_reload: true,
            source: None,
        });
    }

    /// Processes completed loads and emits ECS events. Call once per frame.
    /// If a hot-reload watcher is set, also processes file-change events and re-queues loads.
    pub fn update(&mut self, world: &mut World) {
        if let Some(w) = &mut self.watcher {
            for req in w.drain_reload_requests() {
                if let Some((type_id, id)) = self.path_to_handle.get(&req.path).copied() {
                    self.reload_async(req.path, type_id, id);
                }
            }
        }

        let mut applied = 0;
        while applied < self.completion_budget
            && let Ok(item) = self.complete_rx.try_recv()
        {
            applied += 1;
            if !self.kinds.contains_key(&item.id) {
                // Unloaded while the load was in flight: drop the result.
                continue;
            }
            match item.result {
                Ok(boxed) => {
                    if let (Some(storage_box), Some(inserter)) = (
                        self.storages.get_mut(&item.type_id),
                        self.inserters.get(&item.type_id),
                    ) {
                        inserter(storage_box.as_mut(), item.id, boxed);
                        log::trace!("Asset {:?} loaded: {:?}", item.id, item.path);

                        if let Some(ev) =
                            world.get_resource_mut::<redox_ecs::Events<AssetLoadedEvent>>()
                        {
                            ev.send(AssetLoadedEvent {
                                type_id: item.type_id,
                                id: item.id,
                                path: item.path.clone(),
                            });
                        }
                        if item.is_reload
                            && let Some(ev) =
                                world.get_resource_mut::<redox_ecs::Events<AssetReloadedEvent>>()
                        {
                            ev.send(AssetReloadedEvent {
                                type_id: item.type_id,
                                id: item.id,
                                path: item.path.clone(),
                            });
                        }
                    }
                }
                Err(msg) => {
                    if let (Some(storage_box), Some(marker)) = (
                        self.storages.get_mut(&item.type_id),
                        self.failed_markers.get(&item.type_id),
                    ) {
                        marker(storage_box.as_mut(), item.id);
                    }
                    log::error!("Asset {:?} failed: {} ({:?})", item.id, msg, item.path);
                    if let Some(ev) =
                        world.get_resource_mut::<redox_ecs::Events<AssetLoadFailedEvent>>()
                    {
                        ev.send(AssetLoadFailedEvent {
                            type_id: item.type_id,
                            id: item.id,
                            path: item.path,
                            message: msg,
                        });
                    }
                }
            }
        }
    }

    /// Returns the path for a handle if it was loaded from a path (for hot-reload lookup).
    pub fn path_for_handle(&self, type_id: TypeId, id: AssetId) -> Option<&PathBuf> {
        self.path_to_handle
            .iter()
            .find(|(_, (tid, aid))| *tid == type_id && *aid == id)
            .map(|(p, _)| p)
    }

    /// Starts an async load with a custom closure (no path cache). Use for one-off loads.
    pub fn load_async_with<T, F>(&mut self, loader_fn: F) -> Handle<T>
    where
        T: 'static + Send + Sync,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let handle = Handle::<T>::new(AssetId::next());
        self.storage_mut::<T>().mark_loading(handle);
        self.track(handle.id, TypeId::of::<T>());
        let complete_tx = self.complete_tx.clone();
        let id = handle.id;
        let type_id = TypeId::of::<T>();
        std::thread::spawn(move || {
            let result = loader_fn().map(|a| Box::new(a) as Box<dyn Any + Send + Sync>);
            let _ = complete_tx.send(LoadComplete {
                type_id,
                id,
                path: PathBuf::new(),
                is_reload: false,
                result,
            });
        });
        handle
    }
}

// --- lifetime management ---

impl AssetManager {
    fn track(&mut self, id: AssetId, type_id: TypeId) {
        self.kinds.insert(id, type_id);
        self.refs.insert(id, 1);
    }

    /// Number of owners of the asset (0 for unknown or unloaded ids).
    pub fn ref_count(&self, id: AssetId) -> u32 {
        self.refs.get(&id).copied().unwrap_or(0)
    }

    /// Adds an owner.
    pub fn retain<T: 'static + Send + Sync>(&mut self, handle: Handle<T>) {
        if let Some(count) = self.refs.get_mut(&handle.id) {
            *count += 1;
        }
    }

    /// Drops an owner. Returns `true` when that unloaded the asset.
    pub fn release<T: 'static + Send + Sync>(&mut self, handle: Handle<T>) -> bool {
        self.release_id(handle.id)
    }

    /// Declares that `parent` needs `child`: `child` gains an owner now and loses it when
    /// `parent` is unloaded. Returns `false` (and does nothing) for unknown ids.
    pub fn add_dependency(&mut self, parent: AssetId, child: AssetId) -> bool {
        if parent == child || !self.refs.contains_key(&parent) || !self.refs.contains_key(&child) {
            return false;
        }
        let list = self.deps.entry(parent).or_default();
        if list.contains(&child) {
            return true;
        }
        list.push(child);
        *self.refs.entry(child).or_insert(0) += 1;
        true
    }

    /// Direct dependencies of an asset.
    pub fn dependencies(&self, id: AssetId) -> &[AssetId] {
        self.deps.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Status of an asset by id, whatever its type.
    pub fn status_of(&self, id: AssetId) -> AssetStatus {
        let Some(type_id) = self.kinds.get(&id) else {
            return AssetStatus::Failed;
        };
        match (self.storages.get(type_id), self.statusers.get(type_id)) {
            (Some(storage), Some(status)) => status(storage.as_ref(), id),
            _ => AssetStatus::Failed,
        }
    }

    /// `Ready` when the asset and everything it depends on (transitively) is ready, `Loading`
    /// while any part still loads, `Failed` when any part failed or is unknown.
    pub fn status_with_dependencies(&self, id: AssetId) -> AssetStatus {
        let mut stack = vec![id];
        let mut seen = std::collections::HashSet::new();
        let mut loading = false;
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            match self.status_of(current) {
                AssetStatus::Failed => return AssetStatus::Failed,
                AssetStatus::Loading => loading = true,
                AssetStatus::Ready => {}
            }
            stack.extend_from_slice(self.dependencies(current));
        }
        if loading {
            AssetStatus::Loading
        } else {
            AssetStatus::Ready
        }
    }

    fn release_id(&mut self, id: AssetId) -> bool {
        let Some(count) = self.refs.get_mut(&id) else {
            return false;
        };
        *count -= 1;
        if *count > 0 {
            return false;
        }
        self.unload_id(id);
        true
    }

    fn unload_id(&mut self, id: AssetId) {
        self.refs.remove(&id);
        if let Some(type_id) = self.kinds.remove(&id)
            && let (Some(storage), Some(remove)) =
                (self.storages.get_mut(&type_id), self.removers.get(&type_id))
        {
            remove(storage.as_mut(), id);
        }
        self.path_to_handle.retain(|_, (_, asset)| *asset != id);
        for child in self.deps.remove(&id).unwrap_or_default() {
            self.release_id(child);
        }
    }

    /// Number of live (tracked) assets.
    pub fn live_assets(&self) -> usize {
        self.kinds.len()
    }
}

impl Default for AssetManager {
    fn default() -> Self {
        Self::new("assets")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_retrieve() {
        let mut mgr = AssetManager::new(".");
        let handle = mgr.insert("test_asset".to_string());
        assert_eq!(mgr.status(handle), AssetStatus::Ready);
        assert_eq!(mgr.get(handle).unwrap(), "test_asset");
    }

    #[test]
    fn multiple_types() {
        let mut mgr = AssetManager::new(".");
        let h1 = mgr.insert(42u32);
        let h2 = mgr.insert("hello".to_string());
        assert_eq!(*mgr.get(h1).unwrap(), 42u32);
        assert_eq!(mgr.get(h2).unwrap(), "hello");
    }

    #[test]
    fn missing_asset() {
        let mgr = AssetManager::new(".");
        let handle = Handle::<String>::new(AssetId::next());
        assert_eq!(mgr.status(handle), AssetStatus::Failed);
        assert!(mgr.get(handle).is_none());
    }
}

//! Central asset manager — ECS resource for loading and accessing assets.
//!
//! Register loaders with [`register_loader`](AssetManager::register_loader), then use
//! [`load_async`](AssetManager::load_async) to load by path. Call [`update`](AssetManager::update)
//! each frame to process completed loads and emit ECS events.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::io::Read;

use flume::{Receiver, Sender};
use redox_ecs::World;

use crate::error::AssetError;
use crate::event::{AssetLoadedEvent, AssetLoadFailedEvent, AssetReloadedEvent};
use crate::handle::{AssetId, AssetStatus, Handle};
use crate::hot_reload::HotReloadWatcher;
use crate::loader::AssetLoader;
use crate::storage::AssetStorage;

/// Task sent to the loader thread.
struct LoadTask {
    type_id: TypeId,
    id: AssetId,
    path: PathBuf,
    is_reload: bool,
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
    fn load(&self, bytes: &[u8]) -> Result<Box<dyn Any + Send + Sync>, crate::error::AssetError>;
    #[allow(dead_code)]
    fn extensions(&self) -> Vec<&'static str>;
}

struct LoaderWrapper<L: AssetLoader> {
    inner: L,
}

impl<L: AssetLoader> LoaderErased for LoaderWrapper<L> {
    fn load(&self, bytes: &[u8]) -> Result<Box<dyn Any + Send + Sync>, crate::error::AssetError> {
        self.inner.load(bytes).map(|a| Box::new(a) as Box<dyn Any + Send + Sync>)
    }
    fn extensions(&self) -> Vec<&'static str> {
        self.inner.extensions()
    }
}

type LoaderRegistry = Arc<RwLock<HashMap<TypeId, Box<dyn LoaderErased>>>>;

fn worker_loop(registry: LoaderRegistry, rx: Receiver<LoadTask>, tx: Sender<LoadComplete>) {
    while let Ok(task) = rx.recv() {
        let bytes = match std::fs::File::open(&task.path).and_then(|mut f| {
            let mut v = Vec::new();
            f.read_to_end(&mut v).map(|_| v)
        }) {
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
                Some(loader) => loader.load(&bytes).map_err(|e| e.to_string()),
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
}

impl AssetManager {
    /// Creates a new asset manager with the given base path.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        let (task_tx, task_rx) = flume::unbounded();
        let (complete_tx, complete_rx) = flume::unbounded();
        let loaders: LoaderRegistry = Arc::new(RwLock::new(HashMap::new()));
        let registry_clone = Arc::clone(&loaders);
        let complete_tx_worker = complete_tx.clone();
        std::thread::spawn(move || worker_loop(registry_clone, task_rx, complete_tx_worker));

        Self {
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
        }
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

        self.inserters
            .entry(type_id)
            .or_insert(|any, id, boxed| {
                if let Some(s) = any.downcast_mut::<AssetStorage<T>>() {
                    if let Ok(asset) = boxed.downcast::<T>() {
                        s.insert(Handle::new(id), *asset);
                    }
                }
            });

        self.failed_markers
            .entry(type_id)
            .or_insert(|any, id| {
                if let Some(s) = any.downcast_mut::<AssetStorage<T>>() {
                    s.mark_failed(Handle::new(id));
                }
            });

        self.storages
            .entry(type_id)
            .or_insert_with(|| Box::new(AssetStorage::<T>::new()));
    }

    /// Returns the storage for type `T`, if it exists.
    pub fn storage<T: 'static + Send + Sync>(&self) -> Option<&AssetStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<AssetStorage<T>>())
    }

    fn storage_mut<T: 'static + Send + Sync>(&mut self) -> &mut AssetStorage<T> {
        let type_id = TypeId::of::<T>();
        if !self.storages.contains_key(&type_id) {
            self.storages
                .insert(type_id, Box::new(AssetStorage::<T>::new()));
        }
        self.storages
            .get_mut(&type_id)
            .and_then(|s| s.downcast_mut::<AssetStorage<T>>())
            .expect("type mismatch")
    }

    /// Inserts an already-loaded asset and returns a handle.
    pub fn insert<T: 'static + Send + Sync>(&mut self, asset: T) -> Handle<T> {
        let handle = Handle::new(AssetId::next());
        self.storage_mut::<T>().insert(handle, asset);
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
        let path = self.base_path.join(path.as_ref());

        let type_id = TypeId::of::<T>();
        if let Some((tid, id)) = self.path_to_handle.get(&path) {
            if *tid == type_id {
                return Ok(Handle::new(*id));
            }
        }

        if !self.loaders.read().map(|r| r.contains_key(&type_id)).unwrap_or(false) {
            return Err(AssetError::UnsupportedFormat(format!(
                "no loader registered for type_id {:?}",
                type_id
            )));
        }

        let id = AssetId::next();
        self.storage_mut::<T>().mark_loading(Handle::new(id));
        self.path_to_handle.insert(path.clone(), (type_id, id));

        let _ = self.task_tx.send(LoadTask {
            type_id,
            id,
            path,
            is_reload: false,
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

        while let Ok(item) = self.complete_rx.try_recv() {
            match item.result {
                Ok(boxed) => {
                    if let (Some(storage_box), Some(inserter)) = (
                        self.storages.get_mut(&item.type_id),
                        self.inserters.get(&item.type_id),
                    ) {
                        inserter(storage_box.as_mut(), item.id, boxed);
                        log::trace!("Asset {:?} loaded: {:?}", item.id, item.path);

                        if let Some(ev) = world.get_resource_mut::<redox_ecs::Events<AssetLoadedEvent>>() {
                            ev.send(AssetLoadedEvent {
                                type_id: item.type_id,
                                id: item.id,
                                path: item.path.clone(),
                            });
                        }
                        if item.is_reload {
                            if let Some(ev) = world.get_resource_mut::<redox_ecs::Events<AssetReloadedEvent>>() {
                                ev.send(AssetReloadedEvent {
                                    type_id: item.type_id,
                                    id: item.id,
                                    path: item.path.clone(),
                                });
                            }
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
                    if let Some(ev) = world.get_resource_mut::<redox_ecs::Events<AssetLoadFailedEvent>>() {
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

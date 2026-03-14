//! Per-type asset storage.

use std::collections::HashMap;
use crate::handle::{AssetId, AssetStatus, Handle};

/// Stores loaded assets of a single type `T` along with their status.
pub struct AssetStorage<T: 'static + Send + Sync> {
    assets: HashMap<AssetId, T>,
    statuses: HashMap<AssetId, AssetStatus>,
}

impl<T: 'static + Send + Sync> AssetStorage<T> {
    /// Creates an empty storage.
    pub fn new() -> Self {
        Self {
            assets: HashMap::new(),
            statuses: HashMap::new(),
        }
    }

    /// Inserts a loaded asset.
    pub fn insert(&mut self, handle: Handle<T>, asset: T) {
        self.assets.insert(handle.id, asset);
        self.statuses.insert(handle.id, AssetStatus::Ready);
    }

    /// Marks an asset as loading (reserves the slot).
    pub fn mark_loading(&mut self, handle: Handle<T>) {
        self.statuses.insert(handle.id, AssetStatus::Loading);
    }

    /// Marks an asset as failed.
    pub fn mark_failed(&mut self, handle: Handle<T>) {
        self.statuses.insert(handle.id, AssetStatus::Failed);
    }

    /// Returns a reference to the asset, if loaded.
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        self.assets.get(&handle.id)
    }

    /// Returns a mutable reference to the asset, if loaded.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.assets.get_mut(&handle.id)
    }

    /// Returns the status of an asset.
    pub fn status(&self, handle: Handle<T>) -> AssetStatus {
        self.statuses
            .get(&handle.id)
            .copied()
            .unwrap_or(AssetStatus::Failed)
    }

    /// Removes an asset from storage.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        self.statuses.remove(&handle.id);
        self.assets.remove(&handle.id)
    }

    /// Number of loaded assets.
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// Whether storage is empty.
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

impl<T: 'static + Send + Sync> Default for AssetStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::AssetId;

    #[test]
    fn insert_and_get() {
        let mut storage = AssetStorage::<String>::new();
        let handle = Handle::new(AssetId::next());
        storage.insert(handle, "hello".to_string());

        assert_eq!(storage.status(handle), AssetStatus::Ready);
        assert_eq!(storage.get(handle).unwrap(), "hello");
        assert_eq!(storage.len(), 1);
    }

    #[test]
    fn loading_then_ready() {
        let mut storage = AssetStorage::<Vec<u8>>::new();
        let handle = Handle::new(AssetId::next());

        storage.mark_loading(handle);
        assert_eq!(storage.status(handle), AssetStatus::Loading);
        assert!(storage.get(handle).is_none());

        storage.insert(handle, vec![1, 2, 3]);
        assert_eq!(storage.status(handle), AssetStatus::Ready);
        assert_eq!(storage.get(handle).unwrap(), &vec![1, 2, 3]);
    }

    #[test]
    fn remove_asset() {
        let mut storage = AssetStorage::<String>::new();
        let handle = Handle::new(AssetId::next());
        storage.insert(handle, "world".to_string());
        assert!(storage.remove(handle).is_some());
        assert!(storage.get(handle).is_none());
        assert_eq!(storage.len(), 0);
    }
}

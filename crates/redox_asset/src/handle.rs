//! Lightweight asset handles and identifiers.
//!
//! [`Handle<T>`] is a copyable, type-safe reference to an asset. Validity and
//! lifetime are determined by the [`AssetManager`](crate::manager::AssetManager);
//! use [`AssetManager::get`] or [`AssetManager::status`] to check if a handle
//! is still valid and loaded.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for unique asset IDs.
static NEXT_ASSET_ID: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a loaded asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssetId(pub u64);

impl AssetId {
    /// Generates a new, globally unique id.
    pub fn next() -> Self {
        Self(NEXT_ASSET_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Status of an asset load operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetStatus {
    /// The asset is currently being loaded.
    Loading,
    /// The asset has been successfully loaded and is ready.
    Ready,
    /// The asset failed to load.
    Failed,
}

/// A lightweight, copyable handle to an asset of type `T`.
///
/// The handle does not own the data — it is an index into the manager's
/// storage. Use [`AssetManager::get`](crate::manager::AssetManager::get) or
/// [`AssetManager::status`](crate::manager::AssetManager::status) to check
/// validity and access the asset.
#[derive(Debug)]
pub struct Handle<T: 'static> {
    /// Unique asset identifier.
    pub id: AssetId,
    _marker: PhantomData<T>,
}

// Manual impls to avoid requiring T: Clone/Copy.
impl<T: 'static> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for Handle<T> {}

impl<T: 'static> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T: 'static> Eq for Handle<T> {}

impl<T: 'static> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

// Handles are thread-safe; they only carry an ID and type marker.
unsafe impl<T: 'static> Send for Handle<T> {}
unsafe impl<T: 'static> Sync for Handle<T> {}

impl<T: 'static> Handle<T> {
    /// Creates a new handle with the given ID.
    pub fn new(id: AssetId) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// Returns the underlying asset ID.
    #[inline]
    pub fn id(&self) -> AssetId {
        self.id
    }

    /// Downgrades this handle to a weak handle.
    #[inline]
    pub fn downgrade(self) -> WeakHandle<T> {
        WeakHandle::new(self.id)
    }
}

/// A weak reference to an asset that does not keep it loaded.
///
/// Use [`Handle::downgrade`] to create. When the asset is unloaded (or never
/// loaded), [`AssetManager::get`](crate::manager::AssetManager::get) with a
/// weak handle's ID will return `None`. In the current implementation assets
/// are not reference-counted, so weak and strong handles behave the same for
/// access; the distinction is semantic for future refcounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeakHandle<T: 'static> {
    id: AssetId,
    _marker: PhantomData<T>,
}

unsafe impl<T: 'static> Send for WeakHandle<T> {}
unsafe impl<T: 'static> Sync for WeakHandle<T> {}

impl<T: 'static> WeakHandle<T> {
    pub fn new(id: AssetId) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    pub fn id(&self) -> AssetId {
        self.id
    }

    /// Tries to upgrade to a strong handle. Returns a handle with the same ID;
    /// validity still depends on the manager.
    pub fn upgrade(self) -> Handle<T> {
        Handle::new(self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_ids() {
        let a = AssetId::next();
        let b = AssetId::next();
        assert_ne!(a, b);
    }

    #[test]
    fn handle_is_copy() {
        let h: Handle<String> = Handle::new(AssetId::next());
        let h2 = h;
        assert_eq!(h.id, h2.id);
    }

    #[test]
    fn downgrade_upgrade() {
        let id = AssetId::next();
        let strong = Handle::<i32>::new(id);
        let weak = strong.downgrade();
        assert_eq!(weak.id(), id);
        let strong2 = weak.upgrade();
        assert_eq!(strong.id, strong2.id);
    }
}

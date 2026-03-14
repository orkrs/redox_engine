//! ECS events emitted by the asset system.
//!
//! Insert [`Events<AssetLoadedEvent>`] and optionally [`Events<AssetReloadedEvent>`]
//! / [`Events<AssetLoadFailedEvent>`] as resources, then run [`AssetManager::update`]
//! each frame. Systems can read these events to react to finished loads or hot-reloads.

use std::path::PathBuf;
use crate::handle::AssetId;
use std::any::TypeId;

/// Type-erased payload for "asset finished loading".
///
/// Subscribe with `Events<AssetLoadedEvent>`. Use `type_id` to match the asset type
/// and `id` to look up the handle in the manager.
#[derive(Debug, Clone)]
pub struct AssetLoadedEvent {
    /// Type of the asset (e.g. `TypeId::of::<image::DynamicImage>()`).
    pub type_id: TypeId,
    /// Asset id (use with the appropriate `Handle<T>`).
    pub id: AssetId,
    /// Path that was loaded (for logging or path-based lookups).
    pub path: PathBuf,
}

/// Emitted when an asset was hot-reloaded (file changed on disk).
#[derive(Debug, Clone)]
pub struct AssetReloadedEvent {
    pub type_id: TypeId,
    pub id: AssetId,
    pub path: PathBuf,
}

/// Emitted when an async load failed.
#[derive(Debug, Clone)]
pub struct AssetLoadFailedEvent {
    pub type_id: TypeId,
    pub id: AssetId,
    pub path: PathBuf,
    pub message: String,
}

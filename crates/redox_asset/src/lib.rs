//! Asset management subsystem for the RedOx Engine.
//!
//! Provides:
//! - Type-safe [`Handle<T>`] and [`WeakHandle<T>`] for loaded assets
//! - Central [`AssetManager`] (ECS resource) with async loading and path cache
//! - [`AssetLoader`] trait and implementations (e.g. [`ImageLoader`])
//! - ECS events: [`AssetLoadedEvent`], [`AssetReloadedEvent`], [`AssetLoadFailedEvent`]
//! - Optional hot-reload via [`HotReloadWatcher`]

pub mod error;
pub mod event;
pub mod handle;
pub mod hot_reload;
pub mod loader;
pub mod manager;
pub mod storage;

pub use error::AssetError;
pub use event::{AssetLoadedEvent, AssetLoadFailedEvent, AssetReloadedEvent};
pub use handle::{AssetId, AssetStatus, Handle, WeakHandle};
pub use hot_reload::{HotReloadWatcher, ReloadRequest};
pub use loader::{AssetLoader, ImageLoader};
pub use manager::AssetManager;
pub use storage::AssetStorage;

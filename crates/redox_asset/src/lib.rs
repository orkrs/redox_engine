//! Asset management subsystem for the RedOx Engine.
//!
//! Provides:
//! - Type-safe [`Handle<T>`] and [`WeakHandle<T>`] for loaded assets
//! - Central [`AssetManager`] (ECS resource) with async loading and path cache
//! - [`AssetLoader`] trait and implementations (e.g. [`ImageLoader`], [`GltfLoader`])
//! - ECS events: [`AssetLoadedEvent`], [`AssetReloadedEvent`], [`AssetLoadFailedEvent`]
//! - Optional hot-reload via [`HotReloadWatcher`]

#![warn(missing_docs)]

pub mod error;
pub mod event;
pub mod guid;
pub mod handle;
pub mod hot_reload;
pub mod loader;
pub mod manager;
pub mod pak;
pub mod storage;

pub use error::AssetError;
pub use event::{AssetLoadFailedEvent, AssetLoadedEvent, AssetReloadedEvent};
pub use guid::{AssetGuid, AssetMeta, AssetRegistry, RegistryError, ScanReport};
pub use handle::{AssetId, AssetStatus, Handle, WeakHandle};
pub use hot_reload::{HotReloadWatcher, ReloadRequest};
pub use loader::{
    AssetLoader, GltfLoader, GltfMaterialData, GltfMeshData, GltfNodeData, GltfSceneData,
    GltfTextureData, ImageLoader, Ktx2Loader, Ktx2Texture, TextureFormat, load_scene_from_path,
};
pub use manager::AssetManager;
pub use pak::{AssetSource, DirSource, PakError, PakReader, PakSource, PakWriter};
pub use storage::AssetStorage;

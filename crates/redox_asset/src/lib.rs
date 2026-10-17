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
pub mod queue;
pub mod storage;
pub mod stream;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use error::AssetError;
pub use event::{AssetLoadFailedEvent, AssetLoadedEvent, AssetReloadedEvent};
pub use guid::{AssetGuid, AssetMeta, AssetRegistry, RegistryError, ScanReport};
pub use handle::{AssetId, AssetStatus, Handle, WeakHandle};
pub use hot_reload::{HotReloadWatcher, ReloadRequest};
pub use loader::{
    AssetLoader, GltfLoader, GltfMaterialData, GltfMeshData, GltfNodeData, GltfSceneData,
    GltfTextureData, ImageLoader, Ktx2Loader, Ktx2Texture, LoadOptions, TextureFormat,
    load_scene_from_path,
};
#[cfg(feature = "basis")]
pub use loader::{BasisTarget, basis_file_format, basis_file_to_ktx2, uastc_dfd};
#[cfg(feature = "encode")]
pub use loader::{EncodeMode, EncodeOptions, EncodeShape, encode_basis, encode_ktx2};
pub use manager::AssetManager;
pub use pak::{AssetSource, DirSource, PakError, PakReader, PakSource, PakWriter};
pub use queue::{Priority, TaskQueue};
pub use storage::AssetStorage;
pub use stream::{MipReady, StreamInfo, StreamedTexture};

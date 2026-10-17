//! Asset loader trait and concrete loaders.
//!
//! Loaders turn raw bytes (from disk or network) into in-memory asset data.
//! The manager is responsible for reading the file and passing the bytes to
//! the appropriate loader based on file extension.

mod gltf_loader;
mod image_loader;
mod ktx2_loader;

pub use gltf_loader::{
    GltfLoader, GltfMaterialData, GltfMeshData, GltfNodeData, GltfSceneData, GltfTextureData,
    load_scene_from_path,
};
pub use image_loader::ImageLoader;
pub use ktx2_loader::{Ktx2Loader, Ktx2Texture, TextureFormat};

use crate::error::AssetError;

/// Trait for loading an asset of type `T` from raw bytes or a filesystem path.
pub trait AssetLoader: Send + Sync + 'static {
    /// The output asset type.
    type Asset: Send + Sync + 'static;

    /// Loads the asset from the given bytes.
    fn load(&self, bytes: &[u8]) -> Result<Self::Asset, AssetError>;

    /// Loads from a filesystem path (default: read bytes already provided).
    /// glTF uses this to resolve sibling `.bin` / `.png` URIs.
    fn load_from_path(
        &self,
        path: &std::path::Path,
        bytes: &[u8],
    ) -> Result<Self::Asset, AssetError> {
        let _ = path;
        self.load(bytes)
    }

    /// File extensions this loader supports (e.g. `["png", "jpg"]`).
    fn extensions(&self) -> Vec<&'static str>;
}

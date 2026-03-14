//! Asset loader trait and concrete loaders.
//!
//! Loaders turn raw bytes (from disk or network) into in-memory asset data.
//! The manager is responsible for reading the file and passing the bytes to
//! the appropriate loader based on file extension.

mod image_loader;

pub use image_loader::ImageLoader;

use crate::error::AssetError;

/// Trait for loading an asset of type `T` from raw bytes.
///
/// The manager resolves the path, reads the file, and calls `load` with the
/// bytes. Implementations must be `Send + Sync` so they can be used from
/// background threads.
pub trait AssetLoader: Send + Sync + 'static {
    /// The output asset type.
    type Asset: Send + Sync + 'static;

    /// Loads the asset from the given bytes.
    fn load(&self, bytes: &[u8]) -> Result<Self::Asset, AssetError>;

    /// File extensions this loader supports (e.g. `["png", "jpg"]`).
    fn extensions(&self) -> Vec<&'static str>;
}

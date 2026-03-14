//! Asset loading and management errors.

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during asset loading or access.
#[derive(Error, Debug)]
pub enum AssetError {
    #[error("I/O error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Image decode error: {0}")]
    Image(#[from] image::ImageError),

    #[error("GLTF load error: {0}")]
    Gltf(String),

    #[error("Unsupported file extension or format: {0}")]
    UnsupportedFormat(String),

    #[error("Asset not found: {0}")]
    NotFound(PathBuf),

    #[error("Asset load failed: {0}")]
    LoadFailed(String),
}

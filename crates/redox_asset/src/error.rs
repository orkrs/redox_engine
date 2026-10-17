//! Asset loading and management errors.

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during asset loading or access.
#[derive(Error, Debug)]
pub enum AssetError {
    /// Reading the file failed.
    #[error("I/O error at {path:?}: {source}")]
    Io {
        /// File that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Decoding an image failed.
    #[error("Image decode error: {0}")]
    Image(#[from] image::ImageError),

    /// Parsing or validating a glTF document failed.
    #[error("GLTF load error: {0}")]
    Gltf(String),

    /// The file extension or content is not a supported format.
    #[error("Unsupported file extension or format: {0}")]
    UnsupportedFormat(String),

    /// The path does not exist.
    #[error("Asset not found: {0}")]
    NotFound(PathBuf),

    /// Loading failed for another reason (message attached).
    #[error("Asset load failed: {0}")]
    LoadFailed(String),
}

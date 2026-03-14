//! Image asset loader using the `image` crate.
//!
//! Produces [`image::DynamicImage`] for use by the renderer (e.g. upload to GPU).

use crate::error::AssetError;
use crate::loader::AssetLoader;

/// Loads images from bytes (PNG, JPEG, etc.) as `image::DynamicImage`.
pub struct ImageLoader;

impl AssetLoader for ImageLoader {
    type Asset = image::DynamicImage;

    fn load(&self, bytes: &[u8]) -> Result<Self::Asset, AssetError> {
        image::load_from_memory(bytes).map_err(AssetError::Image)
    }

    fn extensions(&self) -> Vec<&'static str> {
        vec!["png", "jpg", "jpeg", "bmp", "tga", "tiff", "webp", "hdr"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_include_png() {
        let loader = ImageLoader;
        let exts = loader.extensions();
        assert!(exts.contains(&"png"));
    }
}

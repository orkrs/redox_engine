//! KTX2 texture loader: GPU-ready (block-compressed) textures without CPU decoding.
//!
//! Only containers without supercompression are accepted; the mip levels are handed to the
//! renderer as they are. Basis Universal / Zstandard payloads are reported as unsupported.

use crate::error::AssetError;
use crate::loader::AssetLoader;

/// Pixel format of a [`Ktx2Texture`], limited to what the renderer can upload directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    /// 8-bit RGBA, linear.
    Rgba8Unorm,
    /// 8-bit RGBA, sRGB.
    Rgba8Srgb,
    /// BC1 (DXT1), linear.
    Bc1Unorm,
    /// BC1 (DXT1), sRGB.
    Bc1Srgb,
    /// BC3 (DXT5), linear.
    Bc3Unorm,
    /// BC3 (DXT5), sRGB.
    Bc3Srgb,
    /// BC5 (two-channel normal maps).
    Bc5Unorm,
    /// BC7, linear.
    Bc7Unorm,
    /// BC7, sRGB.
    Bc7Srgb,
}

impl TextureFormat {
    fn from_vk(format: ktx2::Format) -> Option<Self> {
        use ktx2::Format as F;
        Some(match format {
            F::R8G8B8A8_UNORM => Self::Rgba8Unorm,
            F::R8G8B8A8_SRGB => Self::Rgba8Srgb,
            F::BC1_RGBA_UNORM_BLOCK => Self::Bc1Unorm,
            F::BC1_RGBA_SRGB_BLOCK => Self::Bc1Srgb,
            F::BC3_UNORM_BLOCK => Self::Bc3Unorm,
            F::BC3_SRGB_BLOCK => Self::Bc3Srgb,
            F::BC5_UNORM_BLOCK => Self::Bc5Unorm,
            F::BC7_UNORM_BLOCK => Self::Bc7Unorm,
            F::BC7_SRGB_BLOCK => Self::Bc7Srgb,
            _ => return None,
        })
    }

    /// `true` for the block-compressed formats.
    pub fn is_block_compressed(self) -> bool {
        !matches!(self, Self::Rgba8Unorm | Self::Rgba8Srgb)
    }

    /// Bytes per 4x4 block (compressed) or per pixel (RGBA8).
    pub fn bytes_per_block(self) -> usize {
        match self {
            Self::Rgba8Unorm | Self::Rgba8Srgb => 4,
            Self::Bc1Unorm | Self::Bc1Srgb => 8,
            _ => 16,
        }
    }

    /// Size in bytes of one mip level with the given pixel dimensions.
    pub fn level_size(self, width: u32, height: u32) -> usize {
        if self.is_block_compressed() {
            let blocks = |n: u32| n.div_ceil(4).max(1) as usize;
            blocks(width) * blocks(height) * self.bytes_per_block()
        } else {
            width.max(1) as usize * height.max(1) as usize * 4
        }
    }
}

/// A 2D texture with its mip chain, ready for upload.
#[derive(Debug, Clone)]
pub struct Ktx2Texture {
    /// Width of mip 0 in pixels.
    pub width: u32,
    /// Height of mip 0 in pixels.
    pub height: u32,
    /// Pixel format.
    pub format: TextureFormat,
    /// Mip levels, largest first.
    pub levels: Vec<Vec<u8>>,
}

/// Loads `.ktx2` files as [`Ktx2Texture`].
pub struct Ktx2Loader;

fn unsupported(msg: impl Into<String>) -> AssetError {
    AssetError::UnsupportedFormat(msg.into())
}

impl AssetLoader for Ktx2Loader {
    type Asset = Ktx2Texture;

    fn load(&self, bytes: &[u8]) -> Result<Self::Asset, AssetError> {
        let reader =
            ktx2::Reader::new(bytes).map_err(|e| AssetError::LoadFailed(format!("ktx2: {e:?}")))?;
        let header = reader.header();
        if header.supercompression_scheme.is_some() {
            return Err(unsupported("KTX2 supercompression"));
        }
        if header.face_count != 1 || header.layer_count > 1 || header.pixel_depth > 1 {
            return Err(unsupported("KTX2 cubemaps, arrays and 3D textures"));
        }
        let vk = header
            .format
            .ok_or_else(|| unsupported("KTX2 without a Vulkan format (Basis Universal)"))?;
        let format =
            TextureFormat::from_vk(vk).ok_or_else(|| unsupported(format!("KTX2 format {vk:?}")))?;

        let (width, height) = (header.pixel_width, header.pixel_height.max(1));
        let mut levels = Vec::new();
        for (index, level) in reader.levels().enumerate() {
            let data = level.data;
            let mip = |n: u32| (n >> index).max(1);
            let expected = format.level_size(mip(width), mip(height));
            if data.len() != expected {
                return Err(AssetError::LoadFailed(format!(
                    "ktx2 level {index}: {} bytes, expected {expected}",
                    data.len()
                )));
            }
            levels.push(data.to_vec());
        }
        if levels.is_empty() {
            return Err(AssetError::LoadFailed("ktx2 has no mip levels".into()));
        }
        Ok(Ktx2Texture {
            width,
            height,
            format,
            levels,
        })
    }

    fn extensions(&self) -> Vec<&'static str> {
        vec!["ktx2"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KTX2_ID: [u8; 12] = [
        0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
    ];
    const VK_BC7_UNORM: u32 = 145;
    const VK_RGBA8_UNORM: u32 = 37;

    /// Builds a minimal KTX2 container (empty descriptor block, no supercompression global data).
    fn build(vk_format: u32, width: u32, height: u32, scheme: u32, levels: &[Vec<u8>]) -> Vec<u8> {
        let level_index_end = 80 + levels.len() * 24;
        let dfd_offset = level_index_end as u32;
        let data_start = (level_index_end + 4) as u64;
        let mut out = Vec::new();
        out.extend_from_slice(&KTX2_ID);
        for v in [
            vk_format,
            1,
            width,
            height,
            0,
            0,
            1,
            levels.len() as u32,
            scheme,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [dfd_offset, 4, 0, 0] {
            out.extend_from_slice(&v.to_le_bytes()); // dfd + kvd offset/length
        }
        for v in [0u64, 0] {
            out.extend_from_slice(&v.to_le_bytes()); // sgd offset/length
        }
        let mut offset = data_start;
        for level in levels {
            for v in [offset, level.len() as u64, level.len() as u64] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            offset += level.len() as u64;
        }
        out.extend_from_slice(&4u32.to_le_bytes()); // dfd total size
        for level in levels {
            out.extend_from_slice(level);
        }
        out
    }

    #[test]
    fn loads_a_bc7_mip_chain() {
        let levels = vec![vec![1u8; 16 * 4], vec![2u8; 16], vec![3u8; 16]];
        let tex = Ktx2Loader
            .load(&build(VK_BC7_UNORM, 8, 8, 0, &levels))
            .expect("valid ktx2");
        assert_eq!((tex.width, tex.height), (8, 8));
        assert_eq!(tex.format, TextureFormat::Bc7Unorm);
        assert_eq!(tex.levels, levels);
    }

    #[test]
    fn loads_uncompressed_rgba8() {
        let levels = vec![vec![9u8; 4 * 4 * 4], vec![8u8; 2 * 2 * 4], vec![7u8; 4]];
        let tex = Ktx2Loader
            .load(&build(VK_RGBA8_UNORM, 4, 4, 0, &levels))
            .expect("valid ktx2");
        assert_eq!(tex.format, TextureFormat::Rgba8Unorm);
        assert_eq!(tex.levels.len(), 3);
    }

    #[test]
    fn rejects_wrong_level_sizes_supercompression_and_garbage() {
        let bad_size = build(VK_BC7_UNORM, 8, 8, 0, &[vec![0u8; 15]]);
        assert!(Ktx2Loader.load(&bad_size).is_err());
        let zstd = build(VK_BC7_UNORM, 4, 4, 2, &[vec![0u8; 16]]);
        assert!(matches!(
            Ktx2Loader.load(&zstd),
            Err(AssetError::UnsupportedFormat(_))
        ));
        assert!(Ktx2Loader.load(b"not a ktx2 file").is_err());
        assert!(Ktx2Loader.load(&[]).is_err());
    }

    #[test]
    fn level_sizes_follow_block_math() {
        assert_eq!(TextureFormat::Bc1Unorm.level_size(5, 5), 4 * 8);
        assert_eq!(TextureFormat::Bc7Srgb.level_size(1, 1), 16);
        assert_eq!(TextureFormat::Rgba8Srgb.level_size(3, 2), 24);
    }
}

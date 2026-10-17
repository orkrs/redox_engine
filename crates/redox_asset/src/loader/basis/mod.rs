//! Basis Universal textures: `.basis` files and KTX2 files with ETC1S (BasisLZ) or UASTC data.
//!
//! The data is transcoded on the loading thread to a GPU format the device supports, chosen by
//! [`LoadOptions::basis_target`]: BC7/BC3/BC1 for desktop GPUs, or plain RGBA8 anywhere.
//! Transcoding itself is done by the Basis Universal library (`basis-universal` crate).

mod file;
mod ktx2_bridge;

use basis_universal::{
    BasisTextureFormat, BasisTextureType, TranscodeParameters, Transcoder, TranscoderTextureFormat,
};

use super::ktx2_loader::{Ktx2Texture, TextureFormat};
use crate::error::AssetError;

pub use ktx2_bridge::{basis_file_to_ktx2, uastc_dfd};
pub(crate) use ktx2_bridge::{basislz_to_basis, uastc_to_basis};

/// Which GPU format Basis data is converted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BasisTarget {
    /// Uncompressed RGBA8: works on every device, four to eight times the memory.
    #[default]
    Rgba8,
    /// BC7: best quality for desktop GPUs (needs `TEXTURE_COMPRESSION_BC`).
    Bc7,
    /// BC3 (DXT5): colour and alpha.
    Bc3,
    /// BC1 (DXT1): colour only; textures with alpha use BC3 instead.
    Bc1,
}

/// Options for [`Ktx2Loader`](super::Ktx2Loader).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadOptions {
    /// Target format for Basis data (other data is loaded as stored).
    pub basis_target: BasisTarget,
    /// Treat `.basis` colour data as sRGB. KTX2 files state their transfer function themselves.
    pub srgb: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            basis_target: BasisTarget::Rgba8,
            srgb: true,
        }
    }
}

fn bad(msg: impl Into<String>) -> AssetError {
    AssetError::LoadFailed(format!("basis: {}", msg.into()))
}

/// Largest texture edge accepted (guards against absurd headers).
const MAX_EDGE: u32 = 16_384;

/// Whether `bytes` starts like a `.basis` file.
pub(super) fn is_basis_file(bytes: &[u8]) -> bool {
    bytes.len() > 4 && bytes[0] == b's' && bytes[1] == b'B'
}

/// The concrete output format for a Basis texture.
#[derive(Clone, Copy)]
struct Output {
    format: TextureFormat,
    texture: TranscoderTextureFormat,
}

fn choose_output(options: LoadOptions, width: u32, height: u32, alpha: bool, srgb: bool) -> Output {
    use TextureFormat as F;
    let compressed_ok = width.is_multiple_of(4) && height.is_multiple_of(4);
    let target = match options.basis_target {
        BasisTarget::Bc1 if alpha => BasisTarget::Bc3,
        other => other,
    };
    let pick = |linear, srgb_format, texture| Output {
        format: if srgb { srgb_format } else { linear },
        texture,
    };
    match target {
        BasisTarget::Bc7 if compressed_ok => {
            pick(F::Bc7Unorm, F::Bc7Srgb, TranscoderTextureFormat::BC7_RGBA)
        }
        BasisTarget::Bc3 if compressed_ok => {
            pick(F::Bc3Unorm, F::Bc3Srgb, TranscoderTextureFormat::BC3_RGBA)
        }
        BasisTarget::Bc1 if compressed_ok => {
            pick(F::Bc1Unorm, F::Bc1Srgb, TranscoderTextureFormat::BC1_RGB)
        }
        _ => pick(F::Rgba8Unorm, F::Rgba8Srgb, TranscoderTextureFormat::RGBA32),
    }
}

/// Transcodes a `.basis` file (ETC1S or UASTC) with the Basis Universal library.
pub(super) fn decode_basis_file(
    bytes: &[u8],
    options: LoadOptions,
    srgb_override: Option<bool>,
) -> Result<Ktx2Texture, AssetError> {
    let mut transcoder = Transcoder::new();
    if !transcoder.validate_header(bytes) {
        return Err(bad("invalid header"));
    }
    // CRC16 over header and data: rejects truncated and bit-flipped files before the C++
    // transcoder sees them (it assumes the slice data it is given is not corrupt).
    if !transcoder.validate_file_checksums(bytes, true) {
        return Err(bad("checksum mismatch (the file is damaged)"));
    }
    let images = transcoder.image_count(bytes);
    if images == 0 || images > 65_536 {
        return Err(bad(format!("{images} images")));
    }
    let (layers, faces) = match transcoder.basis_texture_type(bytes) {
        BasisTextureType::TextureType2D if images == 1 => (1, 1),
        BasisTextureType::TextureType2D | BasisTextureType::TextureType2DArray => (images, 1),
        BasisTextureType::TextureTypeCubemapArray if images.is_multiple_of(6) => (images / 6, 6),
        other => {
            return Err(AssetError::UnsupportedFormat(format!(
                "basis texture type {other:?}"
            )));
        }
    };
    let first = transcoder
        .image_info(bytes, 0)
        .ok_or_else(|| bad("missing image info"))?;
    let (width, height, mips) = (
        first.m_orig_width,
        first.m_orig_height,
        first.m_total_levels,
    );
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || mips == 0 || mips > 16
    {
        return Err(bad(format!("{width}x{height} with {mips} levels")));
    }
    let mut alpha = false;
    for image in 0..images {
        let info = transcoder
            .image_info(bytes, image)
            .ok_or_else(|| bad("missing image info"))?;
        if (info.m_orig_width, info.m_orig_height, info.m_total_levels) != (width, height, mips) {
            return Err(bad(
                "all images of an array must have the same size and levels",
            ));
        }
        alpha |= info.m_alpha_flag;
    }

    let mut output = choose_output(
        options,
        width,
        height,
        alpha,
        srgb_override.unwrap_or(options.srgb),
    );
    if !transcoder
        .basis_texture_format(bytes)
        .can_transcode_to_format(output.texture)
    {
        output = choose_output(
            LoadOptions {
                basis_target: BasisTarget::Rgba8,
                ..options
            },
            width,
            height,
            alpha,
            srgb_override.unwrap_or(options.srgb),
        );
    }

    transcoder
        .prepare_transcoding(bytes)
        .map_err(|()| bad("cannot start transcoding"))?;
    let mut texture = Ktx2Texture {
        width,
        height,
        format: output.format,
        layers,
        faces,
        levels: Vec::new(),
    };
    for level in 0..mips {
        let expected = texture.subimage_size(level as usize);
        let mut data = Vec::with_capacity(expected * images as usize);
        for image in 0..images {
            let sub = transcoder
                .transcode_image_level(
                    bytes,
                    output.texture,
                    TranscodeParameters {
                        image_index: image,
                        level_index: level,
                        ..TranscodeParameters::default()
                    },
                )
                .map_err(|e| bad(format!("image {image} level {level}: {e:?}")))?;
            if sub.len() < expected {
                return Err(bad(format!(
                    "image {image} level {level}: {} bytes, expected {expected}",
                    sub.len()
                )));
            }
            data.extend_from_slice(&sub[..expected]);
        }
        texture.levels.push(data);
    }
    transcoder.end_transcoding();
    Ok(texture)
}

/// The Basis format of a `.basis` file, for tools.
pub fn basis_file_format(bytes: &[u8]) -> Option<BasisTextureFormat> {
    let transcoder = Transcoder::new();
    transcoder
        .validate_header(bytes)
        .then(|| transcoder.basis_texture_format(bytes))
}

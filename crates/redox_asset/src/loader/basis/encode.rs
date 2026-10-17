//! Encoding images to Basis Universal / KTX2 (offline tooling; feature `encode`).
//!
//! The result is a KTX2 file with UASTC (optionally Zstandard compressed) or ETC1S (BasisLZ)
//! data, including a full mip chain, which [`Ktx2Loader`](crate::Ktx2Loader) transcodes at
//! load time to whatever the GPU supports.

use basis_universal::{
    BasisTextureFormat, ColorSpace, Compressor, CompressorParams, ETC1S_QUALITY_DEFAULT,
    UASTC_QUALITY_DEFAULT,
};

use super::file::{BasisFile, FLAG_SRGB, TYPE_2D, TYPE_2D_ARRAY, TYPE_CUBEMAP_ARRAY};
use super::ktx2_bridge::basis_file_to_ktx2;
use crate::error::AssetError;

/// Which Basis format to produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeMode {
    /// High quality, 8 bits per texel before Zstandard (good for normals and detailed colour).
    Uastc,
    /// Small files (about 1 bit per texel), visibly lossy; good for large diffuse textures.
    Etc1s,
}

/// How several input images form one texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeShape {
    /// One image.
    Flat,
    /// All images are array layers.
    Array,
    /// 6 images per cube (+X, -X, +Y, -Y, +Z, -Z), any number of cubes.
    Cube,
}

/// Settings for [`encode_ktx2`].
#[derive(Clone, Copy, Debug)]
pub struct EncodeOptions {
    /// Basis format.
    pub mode: EncodeMode,
    /// Whether colours are sRGB (albedo) or linear data (normals, masks).
    pub srgb: bool,
    /// Generate a full mip chain.
    pub mipmaps: bool,
    /// Shape of the result.
    pub shape: EncodeShape,
    /// Compress UASTC data with Zstandard (ignored for ETC1S, which has its own compression).
    pub zstd: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            mode: EncodeMode::Uastc,
            srgb: true,
            mipmaps: true,
            shape: EncodeShape::Flat,
            zstd: true,
        }
    }
}

/// Encodes `images` to a `.basis` file.
pub fn encode_basis(
    images: &[image::DynamicImage],
    options: EncodeOptions,
) -> Result<Vec<u8>, AssetError> {
    let bad = |m: String| AssetError::LoadFailed(format!("basis encoder: {m}"));
    let first = images
        .first()
        .ok_or_else(|| bad("no input images".into()))?;
    let (width, height) = (first.width(), first.height());
    match options.shape {
        EncodeShape::Flat if images.len() != 1 => {
            return Err(bad("a flat texture takes one image".into()));
        }
        EncodeShape::Cube if !images.len().is_multiple_of(6) => {
            return Err(bad(format!(
                "{} images do not form cube maps",
                images.len()
            )));
        }
        _ => {}
    }
    if images
        .iter()
        .any(|i| (i.width(), i.height()) != (width, height))
    {
        return Err(bad("all images must have the same size".into()));
    }
    if options.shape == EncodeShape::Cube && width != height {
        return Err(bad("cube faces must be square".into()));
    }

    let mut params = CompressorParams::new();
    params.set_basis_format(match options.mode {
        EncodeMode::Uastc => BasisTextureFormat::UASTC4x4,
        EncodeMode::Etc1s => BasisTextureFormat::ETC1S,
    });
    params.set_etc1s_quality_level(ETC1S_QUALITY_DEFAULT);
    params.set_uastc_quality_level(UASTC_QUALITY_DEFAULT);
    let space = if options.srgb {
        ColorSpace::Srgb
    } else {
        ColorSpace::Linear
    };
    params.set_color_space(space);
    params.set_mip_color_space(space);
    params.set_generate_mipmaps(options.mipmaps);
    for (index, image) in images.iter().enumerate() {
        let rgba = image.to_rgba8();
        params
            .source_image_mut(index as u32)
            .init(rgba.as_raw(), width, height, 4);
    }

    let threads = std::thread::available_parallelism()
        .map_or(2, |n| n.get() as u32)
        .max(2);
    let mut compressor = Compressor::new(threads);
    // SAFETY: `params` is fully initialised above (images, format, quality) and outlives the
    // `init` call; the compressor copies what it needs.
    unsafe {
        compressor.init(&params);
        compressor.process().map_err(|e| bad(format!("{e:?}")))?;
    }
    let bytes = compressor.basis_file().to_vec();

    // The encoder always writes "array of 2D images"; mark cube maps explicitly.
    let mut file = BasisFile::parse(&bytes)?;
    // UASTC files carry the sRGB flag whatever was asked for; state the intent explicitly.
    file.flags = (file.flags & !FLAG_SRGB) | if options.srgb { FLAG_SRGB } else { 0 };
    file.tex_type = match options.shape {
        EncodeShape::Flat => TYPE_2D,
        EncodeShape::Array => TYPE_2D_ARRAY,
        EncodeShape::Cube => TYPE_CUBEMAP_ARRAY,
    };
    Ok(file.build())
}

/// Encodes `images` to a KTX2 file (see [`EncodeOptions`]).
pub fn encode_ktx2(
    images: &[image::DynamicImage],
    options: EncodeOptions,
) -> Result<Vec<u8>, AssetError> {
    basis_file_to_ktx2(&encode_basis(images, options)?, options.zstd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AssetLoader, BasisTarget, Ktx2Loader, LoadOptions, TextureFormat};

    fn gradient(size: u32, seed: u8) -> image::DynamicImage {
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(size, size, |x, y| {
            image::Rgba([
                (x * 255 / size) as u8,
                (y * 255 / size) as u8,
                seed.wrapping_add(((x ^ y) * 4) as u8),
                255,
            ])
        }))
    }

    fn load(bytes: &[u8]) -> crate::Ktx2Texture {
        Ktx2Loader::with_options(LoadOptions {
            basis_target: BasisTarget::Rgba8,
            srgb: false,
        })
        .load(bytes)
        .expect("encoded file loads")
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        let sum: f64 = a
            .iter()
            .zip(b)
            .map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2))
            .sum();
        10.0 * (255.0f64 * 255.0 / (sum / a.len() as f64).max(1e-9)).log10()
    }

    #[test]
    fn uastc_and_etc1s_round_trip_with_a_mip_chain() {
        let source = gradient(64, 10);
        let reference = source.to_rgba8();
        for (mode, zstd, min_db) in [
            (EncodeMode::Uastc, true, 30.0),
            (EncodeMode::Uastc, false, 30.0),
            (EncodeMode::Etc1s, false, 22.0),
        ] {
            let options = EncodeOptions {
                mode,
                zstd,
                srgb: false,
                ..EncodeOptions::default()
            };
            let ktx2 = encode_ktx2(std::slice::from_ref(&source), options).unwrap();
            let tex = load(&ktx2);
            assert_eq!(
                (tex.width, tex.height, tex.levels.len()),
                (64, 64, 7),
                "{mode:?}"
            );
            assert_eq!(tex.format, TextureFormat::Rgba8Unorm);
            let db = psnr(&tex.levels[0], reference.as_raw());
            assert!(db > min_db, "{mode:?} zstd={zstd}: {db:.1} dB");
        }
    }

    #[test]
    fn srgb_flag_reaches_the_texture_format() {
        let ktx2 = encode_ktx2(&[gradient(16, 0)], EncodeOptions::default()).unwrap();
        let tex = Ktx2Loader::default().load(&ktx2).unwrap();
        assert_eq!(tex.format, TextureFormat::Rgba8Srgb);
    }

    #[test]
    fn cube_maps_and_arrays_keep_their_shape_and_face_order() {
        let faces: Vec<_> = (0..6).map(|i| gradient(16, i * 40)).collect();
        let cube = load(
            &encode_ktx2(
                &faces,
                EncodeOptions {
                    shape: EncodeShape::Cube,
                    srgb: false,
                    ..EncodeOptions::default()
                },
            )
            .unwrap(),
        );
        assert!(cube.is_cube());
        assert_eq!((cube.layers, cube.faces), (1, 6));
        // Each face decodes close to the image given for it, so the order survived.
        for (face, source) in faces.iter().enumerate() {
            let sub = cube.subimage(0, 0, face as u32).unwrap();
            assert!(psnr(sub, source.to_rgba8().as_raw()) > 28.0, "face {face}");
        }

        let array = load(
            &encode_ktx2(
                &faces[..3],
                EncodeOptions {
                    shape: EncodeShape::Array,
                    srgb: false,
                    ..EncodeOptions::default()
                },
            )
            .unwrap(),
        );
        assert_eq!((array.layers, array.faces), (3, 1));
    }

    #[test]
    fn bad_inputs_are_reported() {
        let one = gradient(16, 0);
        assert!(encode_basis(&[], EncodeOptions::default()).is_err());
        let two = [one.clone(), one.clone()];
        assert!(
            encode_basis(&two, EncodeOptions::default()).is_err(),
            "flat takes one image"
        );
        let cube = EncodeOptions {
            shape: EncodeShape::Cube,
            ..EncodeOptions::default()
        };
        assert!(encode_basis(&two, cube).is_err(), "cube needs 6 faces");
        let mixed = [gradient(16, 0), gradient(8, 0)];
        let array = EncodeOptions {
            shape: EncodeShape::Array,
            ..EncodeOptions::default()
        };
        assert!(encode_basis(&mixed, array).is_err(), "sizes must match");
    }
}

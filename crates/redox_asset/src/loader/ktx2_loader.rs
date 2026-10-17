//! KTX2 texture loader: GPU-ready (block-compressed) textures without CPU decoding.
//!
//! Block-compressed and RGBA containers are handed to the renderer as they are (Zstandard is
//! unpacked); Basis Universal data is transcoded, see [`LoadOptions`].

use crate::error::AssetError;
use crate::loader::AssetLoader;
#[cfg(feature = "basis")]
pub use crate::loader::basis::LoadOptions;

/// Placeholder options (Basis support is disabled in this build).
#[cfg(not(feature = "basis"))]
#[derive(Default, Clone, Copy, Debug)]
pub struct LoadOptions;

/// Pixel format of a [`Ktx2Texture`], limited to what the renderer can upload directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    /// 8-bit RGBA, linear.
    Rgba8Unorm,
    /// 8-bit RGBA, sRGB.
    Rgba8Srgb,
    /// 16-bit float RGBA (HDR; used for baked environment maps).
    Rgba16Float,
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
    /// The Vulkan format number stored in KTX2 files.
    pub fn vk_code(self) -> u32 {
        match self {
            Self::Rgba8Unorm => 37,
            Self::Rgba8Srgb => 43,
            Self::Rgba16Float => 97,
            Self::Bc1Unorm => 133,
            Self::Bc1Srgb => 134,
            Self::Bc3Unorm => 137,
            Self::Bc3Srgb => 138,
            Self::Bc5Unorm => 141,
            Self::Bc7Unorm => 145,
            Self::Bc7Srgb => 146,
        }
    }

    pub(crate) fn from_vk(format: ktx2::Format) -> Option<Self> {
        use ktx2::Format as F;
        Some(match format {
            F::R8G8B8A8_UNORM => Self::Rgba8Unorm,
            F::R8G8B8A8_SRGB => Self::Rgba8Srgb,
            F::R16G16B16A16_SFLOAT => Self::Rgba16Float,
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
        !matches!(self, Self::Rgba8Unorm | Self::Rgba8Srgb | Self::Rgba16Float)
    }

    /// Bytes per 4x4 block (compressed) or per pixel (uncompressed formats).
    pub fn bytes_per_block(self) -> usize {
        match self {
            Self::Rgba8Unorm | Self::Rgba8Srgb => 4,
            Self::Rgba16Float => 8,
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
            width.max(1) as usize * height.max(1) as usize * self.bytes_per_block()
        }
    }
}

/// Most array layers accepted in a Basis Universal KTX2 file.
#[cfg(feature = "basis")]
const MAX_BASIS_LAYERS: u32 = 4096;
/// Most mip levels accepted in a Basis Universal KTX2 file.
#[cfg(feature = "basis")]
const MAX_BASIS_LEVELS: usize = 16;

/// Hard limit for the decoded size of one mip level (guards against hostile headers).
pub const MAX_LEVEL_BYTES: usize = 1 << 30;

/// A texture with its mip chain, ready for upload: a plain 2D image, a 2D array, a cube map or a
/// cube array.
///
/// Every entry of `levels` holds all sub-images of that mip, ordered layer, then face (the KTX2
/// layout). Use [`Self::subimage`] to get one of them.
#[derive(Debug, Clone)]
pub struct Ktx2Texture {
    /// Width of mip 0 in pixels.
    pub width: u32,
    /// Height of mip 0 in pixels.
    pub height: u32,
    /// Pixel format.
    pub format: TextureFormat,
    /// Array layers (1 for a single image or a single cube).
    pub layers: u32,
    /// Faces per layer: 1, or 6 for cube maps.
    pub faces: u32,
    /// Mip levels, largest first.
    pub levels: Vec<Vec<u8>>,
}

impl Ktx2Texture {
    /// A single 2D image.
    pub fn new_2d(width: u32, height: u32, format: TextureFormat, levels: Vec<Vec<u8>>) -> Self {
        Self {
            width,
            height,
            format,
            layers: 1,
            faces: 1,
            levels,
        }
    }

    /// Serializes the texture as a KTX2 file, optionally compressing every level with
    /// Zstandard.
    pub fn to_ktx2_bytes(&self, zstd: bool) -> Vec<u8> {
        write_container(&ContainerParams {
            format: self.format,
            width: self.width,
            height: self.height,
            layers: if self.layers > 1 { self.layers } else { 0 },
            faces: self.faces,
            levels: &self.levels,
            zstd,
            declared_sizes: &[],
            vk_override: None,
        })
    }

    /// Whether the faces form a cube map.
    pub fn is_cube(&self) -> bool {
        self.faces == 6
    }

    /// Number of sub-images per mip (`layers * faces`).
    pub fn subimage_count(&self) -> u32 {
        self.layers * self.faces
    }

    /// Size in pixels of mip `mip`.
    pub fn mip_size(&self, mip: usize) -> (u32, u32) {
        ((self.width >> mip).max(1), (self.height >> mip).max(1))
    }

    /// Bytes of one sub-image at mip `mip`.
    pub fn subimage_size(&self, mip: usize) -> usize {
        let (w, h) = self.mip_size(mip);
        self.format.level_size(w, h)
    }

    /// The bytes of one sub-image, or `None` when the indices are out of range.
    pub fn subimage(&self, mip: usize, layer: u32, face: u32) -> Option<&[u8]> {
        if layer >= self.layers || face >= self.faces {
            return None;
        }
        let size = self.subimage_size(mip);
        let start = (layer * self.faces + face) as usize * size;
        self.levels.get(mip)?.get(start..start + size)
    }
}

/// Everything needed to lay out a KTX2 file (shared by the writer and the test builder).
pub(crate) struct ContainerParams<'a> {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    /// Value of the `layerCount` field (0 = not an array).
    pub layers: u32,
    pub faces: u32,
    pub levels: &'a [Vec<u8>],
    pub zstd: bool,
    /// Per-level override of the uncompressed size written to the level index.
    pub declared_sizes: &'a [Option<u64>],
    pub vk_override: Option<u32>,
}

const KTX2_ID: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// Lays out a KTX2 container: header, level index, a minimal descriptor block, level data.
pub(crate) fn write_container(p: &ContainerParams<'_>) -> Vec<u8> {
    let stored: Vec<Vec<u8>> = p
        .levels
        .iter()
        .map(|level| {
            if p.zstd {
                ruzstd::encoding::compress_to_vec(
                    level.as_slice(),
                    ruzstd::encoding::CompressionLevel::Fastest,
                )
            } else {
                level.clone()
            }
        })
        .collect();
    let level_index_end = 80 + stored.len() * 24;
    let data_start = (level_index_end + 4) as u64;

    let mut out = Vec::new();
    out.extend_from_slice(&KTX2_ID);
    for v in [
        p.vk_override.unwrap_or(p.format.vk_code()),
        1,
        p.width,
        p.height,
        0,
        p.layers,
        p.faces,
        stored.len() as u32,
        if p.zstd { 2 } else { 0 },
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [level_index_end as u32, 4, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [0u64, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let mut offset = data_start;
    for (i, level) in stored.iter().enumerate() {
        let uncompressed = p
            .declared_sizes
            .get(i)
            .copied()
            .flatten()
            .unwrap_or(p.levels[i].len() as u64);
        for v in [offset, level.len() as u64, uncompressed] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        offset += level.len() as u64;
    }
    out.extend_from_slice(&4u32.to_le_bytes());
    for level in &stored {
        out.extend_from_slice(level);
    }
    out
}

/// Test hook: lays out a container with overridable header values.
#[cfg(any(test, feature = "testing"))]
pub fn ktx2_container(
    vk_override: Option<u32>,
    format: TextureFormat,
    (width, height): (u32, u32),
    (layers, faces): (u32, u32),
    levels: &[Vec<u8>],
    zstd: bool,
    declared_sizes: &[Option<u64>],
) -> Vec<u8> {
    write_container(&ContainerParams {
        format,
        width,
        height,
        layers,
        faces,
        levels,
        zstd,
        declared_sizes,
        vk_override,
    })
}

/// Loads `.ktx2` and `.basis` files as [`Ktx2Texture`].
///
/// Supports the block formats of [`TextureFormat`] with or without Zstandard supercompression,
/// 2D images, arrays, cube maps and cube arrays. With the `basis` feature it also transcodes
/// Basis Universal data (UASTC and ETC1S/BasisLZ, in KTX2 or `.basis` files) to the format
/// chosen in [`LoadOptions`].
#[derive(Default, Clone, Copy, Debug)]
pub struct Ktx2Loader {
    /// How Basis data is converted.
    pub options: LoadOptions,
}

impl Ktx2Loader {
    /// A loader with the given options (for example `BasisTarget::Bc7` on desktop GPUs).
    pub fn with_options(options: LoadOptions) -> Self {
        Self { options }
    }
}

fn unsupported(msg: impl Into<String>) -> AssetError {
    AssetError::UnsupportedFormat(msg.into())
}

fn bad(msg: impl Into<String>) -> AssetError {
    AssetError::LoadFailed(format!("ktx2: {}", msg.into()))
}

/// Inflates a Zstandard level into exactly `expected` bytes.
pub(crate) fn zstd_decode(
    compressed: &[u8],
    expected: usize,
    index: usize,
) -> Result<Vec<u8>, AssetError> {
    use std::io::Read;

    let mut source = compressed;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(&mut source)
        .map_err(|e| bad(format!("level {index}: zstd header: {e}")))?;
    let mut out = vec![0u8; expected];
    decoder
        .read_exact(&mut out)
        .map_err(|e| bad(format!("level {index}: zstd data: {e}")))?;
    // The frame must end exactly here: more output means the declared size was a lie.
    let mut probe = [0u8; 1];
    match decoder.read(&mut probe) {
        Ok(0) => Ok(out),
        Ok(_) => Err(bad(format!(
            "level {index}: zstd data longer than declared"
        ))),
        Err(e) => Err(bad(format!("level {index}: zstd trailer: {e}"))),
    }
}

impl AssetLoader for Ktx2Loader {
    type Asset = Ktx2Texture;

    fn load(&self, bytes: &[u8]) -> Result<Self::Asset, AssetError> {
        // The parsing crates assume well-formed files in places; turn their panics into errors.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.load_inner(bytes)))
            .unwrap_or_else(|_| Err(bad("the file is malformed (parser panic)")))
    }

    fn extensions(&self) -> Vec<&'static str> {
        vec!["ktx2", "basis"]
    }
}

impl Ktx2Loader {
    fn load_inner(&self, bytes: &[u8]) -> Result<Ktx2Texture, AssetError> {
        #[cfg(feature = "basis")]
        if super::basis::is_basis_file(bytes) {
            return super::basis::decode_basis_file(bytes, self.options, None);
        }

        use ktx2::SupercompressionScheme as Scheme;

        let reader = ktx2::Reader::new(bytes).map_err(|e| bad(format!("{e:?}")))?;
        let header = reader.header();
        if header.pixel_depth > 1 {
            return Err(unsupported("KTX2 3D textures"));
        }
        if header.face_count != 1 && header.face_count != 6 {
            return Err(bad(format!(
                "{} faces (expected 1 or 6)",
                header.face_count
            )));
        }
        if header.face_count == 6 && header.pixel_width != header.pixel_height {
            return Err(bad("cube map faces must be square"));
        }
        let scheme = header.supercompression_scheme;
        let zstd = scheme == Some(Scheme::Zstandard);
        let (width, height) = (header.pixel_width, header.pixel_height.max(1));
        let (layers, faces) = (header.layer_count.max(1), header.face_count);

        let Some(vk) = header.format else {
            #[cfg(feature = "basis")]
            return self.load_basis_ktx2(&reader, zstd);
            #[cfg(not(feature = "basis"))]
            return Err(unsupported(
                "KTX2 without a Vulkan format (Basis Universal)",
            ));
        };
        if scheme.is_some() && !zstd {
            return Err(unsupported(format!("KTX2 supercompression {scheme:?}")));
        }
        let format =
            TextureFormat::from_vk(vk).ok_or_else(|| unsupported(format!("KTX2 format {vk:?}")))?;

        let mut texture = Ktx2Texture {
            width,
            height,
            format,
            layers,
            faces,
            levels: Vec::new(),
        };
        let sizes: Vec<usize> = (0..reader.levels().len())
            .map(|i| texture.subimage_size(i) * texture.subimage_count() as usize)
            .collect();
        texture.levels = read_levels(&reader, zstd, &sizes)?;
        if texture.levels.is_empty() {
            return Err(bad("no mip levels"));
        }
        Ok(texture)
    }
}

/// The payload of every level, decoded from Zstandard when needed and checked against the size
/// each level must have (`expected[i]` bytes).
fn read_levels(
    reader: &ktx2::Reader<&[u8]>,
    zstd: bool,
    expected: &[usize],
) -> Result<Vec<Vec<u8>>, AssetError> {
    let mut levels = Vec::new();
    for (index, level) in reader.levels().enumerate() {
        let expected = *expected.get(index).ok_or_else(|| bad("too many levels"))?;
        if expected > MAX_LEVEL_BYTES {
            return Err(bad(format!("level {index} would need {expected} bytes")));
        }
        levels.push(if zstd {
            if level.uncompressed_byte_length != expected as u64 {
                return Err(bad(format!(
                    "level {index}: declares {} decoded bytes, expected {expected}",
                    level.uncompressed_byte_length
                )));
            }
            zstd_decode(level.data, expected, index)?
        } else {
            if level.data.len() != expected {
                return Err(bad(format!(
                    "level {index}: {} bytes, expected {expected}",
                    level.data.len()
                )));
            }
            level.data.to_vec()
        });
    }
    Ok(levels)
}

#[cfg(feature = "basis")]
impl Ktx2Loader {
    /// KTX2 files whose format is "undefined": Basis Universal UASTC or ETC1S data.
    fn load_basis_ktx2(
        &self,
        reader: &ktx2::Reader<&[u8]>,
        zstd: bool,
    ) -> Result<Ktx2Texture, AssetError> {
        use ktx2::ColorModel;

        let header = reader.header();
        let dfd = reader
            .basic_dfd()
            .ok_or_else(|| unsupported("KTX2 without a format and without a data descriptor"))?;
        let srgb = dfd.transfer_function.is_some_and(|t| t.value() == 2);
        let (width, height) = (header.pixel_width, header.pixel_height.max(1));
        let (layers, faces) = (header.layer_count.max(1), header.face_count);
        if width > 16_384 || height > 16_384 {
            return Err(bad(format!("{width}x{height} is too large")));
        }
        if layers > MAX_BASIS_LAYERS || reader.levels().len() > MAX_BASIS_LEVELS {
            return Err(bad(format!(
                "{layers} layers and {} levels are more than supported",
                reader.levels().len()
            )));
        }
        match dfd.color_model {
            Some(ColorModel::UASTC) => {
                let alpha = dfd
                    .sample_information
                    .first()
                    .is_some_and(|s| matches!(s.channel_type, 3 | 5));
                // Each sub-image is a grid of 16-byte UASTC blocks.
                let sizes: Vec<usize> = (0..reader.levels().len())
                    .map(|i| {
                        let (w, h) = ((width >> i).max(1), (height >> i).max(1));
                        (w.div_ceil(4) * h.div_ceil(4)) as usize * 16 * (layers * faces) as usize
                    })
                    .collect();
                let levels = read_levels(reader, zstd, &sizes)?;
                let basis = super::basis::uastc_to_basis(
                    &levels, width, height, layers, faces, alpha, srgb,
                )?;
                super::basis::decode_basis_file(&basis, self.options, Some(srgb))
            }
            Some(ColorModel::ETC1S) => {
                if header.supercompression_scheme != Some(ktx2::SupercompressionScheme::BasisLZ) {
                    return Err(bad("ETC1S data without BasisLZ supercompression"));
                }
                let payloads: Vec<&[u8]> = reader.levels().map(|l| l.data).collect();
                let basis = super::basis::basislz_to_basis(
                    reader.supercompression_global_data(),
                    &payloads,
                    width,
                    height,
                    layers,
                    faces,
                    srgb,
                )?;
                super::basis::decode_basis_file(&basis, self.options, Some(srgb))
            }
            other => Err(unsupported(format!(
                "KTX2 data with colour model {other:?} and no format"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Ktx2Builder, Supercompression, pattern_byte};

    fn load(builder: &Ktx2Builder) -> Result<Ktx2Texture, AssetError> {
        Ktx2Loader::default().load(&builder.build())
    }

    #[test]
    fn loads_a_bc7_mip_chain() {
        let builder = Ktx2Builder::new(TextureFormat::Bc7Unorm, 8, 8);
        let tex = load(&builder).expect("valid ktx2");
        assert_eq!((tex.width, tex.height), (8, 8));
        assert_eq!(tex.format, TextureFormat::Bc7Unorm);
        assert_eq!(tex.levels.len(), 4);
        assert_eq!(tex.levels, builder.raw_levels());
        assert_eq!((tex.layers, tex.faces), (1, 1));
    }

    #[test]
    fn loads_uncompressed_rgba8() {
        let tex = load(&Ktx2Builder::new(TextureFormat::Rgba8Srgb, 4, 4)).expect("valid ktx2");
        assert_eq!(tex.format, TextureFormat::Rgba8Srgb);
        assert_eq!(tex.levels.len(), 3);
    }

    #[test]
    fn zstd_levels_decode_to_the_same_bytes_as_stored_ones() {
        for format in [
            TextureFormat::Rgba8Unorm,
            TextureFormat::Bc1Srgb,
            TextureFormat::Bc5Unorm,
            TextureFormat::Bc7Unorm,
        ] {
            let plain = Ktx2Builder::new(format, 16, 8);
            let packed = plain.clone().supercompression(Supercompression::Zstd);
            assert_ne!(plain.build(), packed.build());
            let a = load(&plain).unwrap();
            let b = load(&packed).unwrap();
            assert_eq!(a.levels, b.levels, "{format:?}");
        }
    }

    #[test]
    fn zstd_decodes_what_the_reference_encoder_wrote() {
        let raw = Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8);
        let mut file = raw.clone().supercompression(Supercompression::Zstd).build();
        // Replace the first level's frame by one from the reference implementation.
        let mip0 = raw.raw_levels()[0].clone();
        let reference = zstd::bulk::compress(&mip0, 3).unwrap();
        let rebuilt = Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8)
            .mips(1)
            .supercompression(Supercompression::Zstd)
            .level_data(0, mip0.clone())
            .build();
        // Layout: header 80 + one index entry 24 + dfd 4 = 108 bytes, then the frame.
        let data_start = 80 + 24 + 4;
        let mut with_reference = rebuilt[..data_start].to_vec();
        with_reference.extend_from_slice(&reference);
        // Patch the stored length in the level index (offset 8 of the first entry).
        with_reference[80 + 8..80 + 16].copy_from_slice(&(reference.len() as u64).to_le_bytes());
        let tex = Ktx2Loader::default()
            .load(&with_reference)
            .expect("reference frame decodes");
        assert_eq!(tex.levels[0], mip0);
        file.clear();
    }

    #[test]
    fn hostile_zstd_files_fail_cleanly() {
        let packed = Ktx2Builder::new(TextureFormat::Bc7Unorm, 8, 8)
            .supercompression(Supercompression::Zstd);
        // Declared size differs from what the format needs.
        assert!(load(&packed.clone().declare_size(0, 5)).is_err());
        // Declared size is right but the frame holds a different amount of data.
        let short = packed
            .clone()
            .level_data(1, vec![1u8; 8])
            .declare_size(1, 16);
        assert!(load(&short).is_err());
        // Absurd size.
        assert!(load(&packed.clone().declare_size(0, u64::MAX)).is_err());
        // Every single flipped byte either loads or errors, never panics.
        let bytes = packed.build();
        for i in 0..bytes.len() {
            let mut broken = bytes.clone();
            broken[i] ^= 0xA5;
            let _ = Ktx2Loader::default().load(&broken);
        }
        for cut in 0..bytes.len() {
            let _ = Ktx2Loader::default().load(&bytes[..cut]);
        }
    }

    #[test]
    fn rejects_wrong_level_sizes_and_garbage() {
        let bad_size = Ktx2Builder::new(TextureFormat::Bc7Unorm, 8, 8).level_data(0, vec![0u8; 15]);
        assert!(load(&bad_size).is_err());
        assert!(Ktx2Loader::default().load(b"not a ktx2 file").is_err());
        assert!(Ktx2Loader::default().load(&[]).is_err());
        let unknown = Ktx2Builder::new(TextureFormat::Bc7Unorm, 4, 4).vk_format(1000);
        assert!(matches!(
            load(&unknown),
            Err(AssetError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn cube_maps_keep_every_face_and_mip() {
        let tex = load(&Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 8).cube()).unwrap();
        assert!(tex.is_cube());
        assert_eq!((tex.layers, tex.faces, tex.subimage_count()), (1, 6, 6));
        for mip in 0..tex.levels.len() {
            for face in 0..6 {
                let image = tex.subimage(mip, 0, face).unwrap();
                assert_eq!(image.len(), tex.subimage_size(mip));
                assert!(
                    image
                        .iter()
                        .enumerate()
                        .all(|(i, &b)| b == pattern_byte(mip, face, i))
                );
            }
        }
        assert!(tex.subimage(0, 0, 6).is_none() && tex.subimage(0, 1, 0).is_none());
    }

    #[test]
    fn arrays_and_cube_arrays_are_indexed_by_layer_then_face() {
        let array = load(&Ktx2Builder::new(TextureFormat::Bc7Unorm, 8, 8).layers(3)).unwrap();
        assert_eq!((array.layers, array.faces), (3, 1));
        let layer2 = array.subimage(1, 2, 0).unwrap();
        assert!(
            layer2
                .iter()
                .enumerate()
                .all(|(i, &b)| b == pattern_byte(1, 2, i))
        );

        let cubes = load(
            &Ktx2Builder::new(TextureFormat::Rgba8Unorm, 4, 4)
                .cube()
                .layers(2)
                .supercompression(Supercompression::Zstd),
        )
        .unwrap();
        assert_eq!(
            (cubes.layers, cubes.faces, cubes.subimage_count()),
            (2, 6, 12)
        );
        let image = cubes.subimage(0, 1, 3).unwrap();
        assert!(
            image
                .iter()
                .enumerate()
                .all(|(i, &b)| b == pattern_byte(0, 9, i))
        );
    }

    #[test]
    fn cube_faces_must_be_square_and_counts_valid() {
        assert!(load(&Ktx2Builder::new(TextureFormat::Rgba8Unorm, 8, 4).cube()).is_err());
    }

    #[test]
    fn writer_round_trips_through_the_loader() {
        let original = load(
            &Ktx2Builder::new(TextureFormat::Rgba16Float, 8, 8)
                .cube()
                .layers(2),
        )
        .unwrap();
        for zstd in [false, true] {
            let again = Ktx2Loader::default()
                .load(&original.to_ktx2_bytes(zstd))
                .unwrap();
            assert_eq!(again.levels, original.levels, "zstd={zstd}");
            assert_eq!((again.layers, again.faces), (2, 6));
            assert_eq!(again.format, TextureFormat::Rgba16Float);
        }
    }

    #[test]
    fn level_sizes_follow_block_math() {
        assert_eq!(TextureFormat::Bc1Unorm.level_size(5, 5), 4 * 8);
        assert_eq!(TextureFormat::Bc7Srgb.level_size(1, 1), 16);
        assert_eq!(TextureFormat::Rgba8Srgb.level_size(3, 2), 24);
        assert_eq!(TextureFormat::Rgba16Float.level_size(3, 2), 48);
    }
}

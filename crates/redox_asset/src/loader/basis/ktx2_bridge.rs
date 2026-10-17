//! Conversions between `.basis` files and KTX2 containers.
//!
//! * [`basislz_to_basis`] lets the `.basis` transcoder read KTX2 files with BasisLZ (ETC1S)
//!   data: the codebooks and slice payloads are the same bytes, only the wrapper differs.
//! * [`basis_file_to_ktx2`] writes a `.basis` file as KTX2 (UASTC with optional Zstandard, or
//!   ETC1S with BasisLZ), which is what the texture cooking tool produces.

use super::file::{
    BasisFile, FLAG_ETC1S, FLAG_HAS_ALPHA_SLICES, FLAG_SRGB, FORMAT_ETC1S, FORMAT_UASTC,
    SLICE_HAS_ALPHA, Slice, TYPE_2D, TYPE_2D_ARRAY, TYPE_CUBEMAP_ARRAY,
};
use crate::error::AssetError;

const KTX2_ID: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const SCHEME_BASISLZ: u32 = 1;
const SCHEME_ZSTD: u32 = 2;
const COLOR_MODEL_ETC1S: u8 = 163;
const COLOR_MODEL_UASTC: u8 = 166;
const ETC1S_GLOBAL_HEADER: usize = 20;
const ETC1S_IMAGE_DESC: usize = 20;

fn bad(msg: impl Into<String>) -> AssetError {
    AssetError::LoadFailed(format!("ktx2/basis: {}", msg.into()))
}

fn dfd(color_model: u8, srgb: bool, samples: &[(u16, u8, u8)], bytes_plane0: u8) -> Vec<u8> {
    let block_size = 24 + 16 * samples.len() as u32;
    let mut out = Vec::new();
    out.extend_from_slice(&(block_size + 4).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // vendor 0, descriptor type 0 (basic)
    out.extend_from_slice(&((block_size << 16) | 2).to_le_bytes()); // version 2, block size
    out.extend_from_slice(&[color_model, 1, if srgb { 2 } else { 1 }, 0]);
    out.extend_from_slice(&[3, 3, 0, 0]); // 4x4x1 block (stored minus one)
    out.extend_from_slice(&[bytes_plane0, 0, 0, 0, 0, 0, 0, 0]);
    for &(bit_offset, bit_length_minus_one, channel) in samples {
        out.extend_from_slice(&bit_offset.to_le_bytes());
        out.push(bit_length_minus_one);
        out.push(channel);
        out.extend_from_slice(&[0, 0, 0, 0]); // sample position
        out.extend_from_slice(&0u32.to_le_bytes()); // lower
        out.extend_from_slice(&u32::MAX.to_le_bytes()); // upper
    }
    out
}

/// The KTX2 data format descriptor for UASTC 4x4 data.
pub fn uastc_dfd(srgb: bool, alpha: bool) -> Vec<u8> {
    dfd(
        COLOR_MODEL_UASTC,
        srgb,
        &[(0, 127, if alpha { 3 } else { 0 })],
        16,
    )
}

fn etc1s_dfd(srgb: bool, alpha: bool) -> Vec<u8> {
    let mut samples = vec![(0u16, 63u8, 0u8)];
    if alpha {
        samples.push((64, 63, 15));
    }
    dfd(COLOR_MODEL_ETC1S, srgb, &samples, 0)
}

struct Container<'a> {
    width: u32,
    height: u32,
    layers: u32,
    faces: u32,
    scheme: u32,
    dfd: &'a [u8],
    sgd: &'a [u8],
    /// `(stored bytes, uncompressed length)` per mip level, largest first.
    levels: &'a [(Vec<u8>, u64)],
}

fn align8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

fn write_container(c: &Container<'_>) -> Vec<u8> {
    let level_count = c.levels.len();
    let index_end = 80 + level_count * 24;
    let dfd_at = index_end;
    let sgd_at = align8(dfd_at + c.dfd.len());
    let mut data_at = align8(sgd_at + c.sgd.len());

    // Smallest mip first in the file, as the specification recommends.
    let mut offsets = vec![0usize; level_count];
    for (i, (stored, _)) in c.levels.iter().enumerate().rev() {
        offsets[i] = data_at;
        data_at = align8(data_at + stored.len());
    }

    let mut out = vec![0u8; data_at];
    out[..12].copy_from_slice(&KTX2_ID);
    let put32 =
        |out: &mut Vec<u8>, at: usize, v: u32| out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let put64 =
        |out: &mut Vec<u8>, at: usize, v: u64| out[at..at + 8].copy_from_slice(&v.to_le_bytes());
    put32(&mut out, 12, 0); // vkFormat: undefined
    put32(&mut out, 16, 1); // typeSize
    put32(&mut out, 20, c.width);
    put32(&mut out, 24, c.height);
    put32(&mut out, 28, 0);
    put32(&mut out, 32, if c.layers > 1 { c.layers } else { 0 });
    put32(&mut out, 36, c.faces);
    put32(&mut out, 40, level_count as u32);
    put32(&mut out, 44, c.scheme);
    put32(&mut out, 48, dfd_at as u32);
    put32(&mut out, 52, c.dfd.len() as u32);
    put32(&mut out, 56, 0);
    put32(&mut out, 60, 0);
    put64(
        &mut out,
        64,
        if c.sgd.is_empty() { 0 } else { sgd_at as u64 },
    );
    put64(&mut out, 72, c.sgd.len() as u64);
    for (i, (stored, uncompressed)) in c.levels.iter().enumerate() {
        let at = 80 + i * 24;
        put64(&mut out, at, offsets[i] as u64);
        put64(&mut out, at + 8, stored.len() as u64);
        put64(&mut out, at + 16, *uncompressed);
        out[offsets[i]..offsets[i] + stored.len()].copy_from_slice(stored);
    }
    out[dfd_at..dfd_at + c.dfd.len()].copy_from_slice(c.dfd);
    out[sgd_at..sgd_at + c.sgd.len()].copy_from_slice(c.sgd);
    out
}

/// Writes a `.basis` file as KTX2. UASTC data may additionally be compressed with Zstandard
/// (`zstd`); ETC1S always uses BasisLZ and ignores the flag.
pub fn basis_file_to_ktx2(basis: &[u8], zstd: bool) -> Result<Vec<u8>, AssetError> {
    let file = BasisFile::parse(basis)?;
    let (layers, faces) = match file.tex_type {
        TYPE_2D if file.images == 1 => (1, 1),
        TYPE_2D | TYPE_2D_ARRAY => (file.images, 1),
        TYPE_CUBEMAP_ARRAY if file.images.is_multiple_of(6) => (file.images / 6, 6),
        other => {
            return Err(bad(format!(
                "texture type {other} cannot be written as KTX2"
            )));
        }
    };
    let levels = file.slices.iter().map(|s| s.level + 1).max().unwrap_or(0);
    let first = file
        .slices
        .iter()
        .find(|s| s.image == 0 && s.level == 0)
        .ok_or_else(|| bad("no base level"))?;
    let (width, height) = (first.width, first.height);
    let srgb = file.flags & FLAG_SRGB != 0;
    let alpha = file.flags & FLAG_HAS_ALPHA_SLICES != 0;
    let slice = |image: u32, level: u32, alpha: bool| -> Result<&Slice, AssetError> {
        file.slices
            .iter()
            .find(|s| {
                s.image == image && s.level == level && (s.flags & SLICE_HAS_ALPHA != 0) == alpha
            })
            .ok_or_else(|| {
                bad(format!(
                    "missing slice (image {image}, level {level}, alpha {alpha})"
                ))
            })
    };

    match file.format {
        FORMAT_UASTC => {
            let mut stored_levels = Vec::new();
            for level in 0..levels {
                let mut raw = Vec::new();
                for image in 0..file.images {
                    // UASTC has one slice per level; its alpha flag only says the data has alpha.
                    let one = file
                        .slices
                        .iter()
                        .find(|s| s.image == image && s.level == level)
                        .ok_or_else(|| {
                            bad(format!("missing slice (image {image}, level {level})"))
                        })?;
                    raw.extend_from_slice(&one.data);
                }
                let uncompressed = raw.len() as u64;
                let stored = if zstd {
                    ruzstd::encoding::compress_to_vec(
                        raw.as_slice(),
                        ruzstd::encoding::CompressionLevel::Fastest,
                    )
                } else {
                    raw
                };
                stored_levels.push((stored, uncompressed));
            }
            Ok(write_container(&Container {
                width,
                height,
                layers,
                faces,
                scheme: if zstd { SCHEME_ZSTD } else { 0 },
                dfd: &uastc_dfd(srgb, alpha),
                sgd: &[],
                levels: &stored_levels,
            }))
        }
        FORMAT_ETC1S => {
            // Global data: header, image descriptors (level-major), then the three blobs.
            let image_count = (levels * file.images) as usize;
            let mut descs = Vec::with_capacity(image_count * ETC1S_IMAGE_DESC);
            let mut stored_levels = Vec::new();
            for level in 0..levels {
                let mut level_data = Vec::new();
                for image in 0..file.images {
                    let rgb = slice(image, level, false)?;
                    let rgb_at = level_data.len() as u32;
                    level_data.extend_from_slice(&rgb.data);
                    let (alpha_at, alpha_len) = if alpha {
                        let a = slice(image, level, true)?;
                        let at = level_data.len() as u32;
                        level_data.extend_from_slice(&a.data);
                        (at, a.data.len() as u32)
                    } else {
                        (0, 0)
                    };
                    for v in [0u32, rgb_at, rgb.data.len() as u32, alpha_at, alpha_len] {
                        descs.extend_from_slice(&v.to_le_bytes());
                    }
                }
                stored_levels.push((level_data, 0));
            }
            let mut sgd = Vec::new();
            sgd.extend_from_slice(&(file.endpoint_count as u16).to_le_bytes());
            sgd.extend_from_slice(&(file.selector_count as u16).to_le_bytes());
            for v in [
                file.endpoints.len(),
                file.selectors.len(),
                file.tables.len(),
                0,
            ] {
                sgd.extend_from_slice(&(v as u32).to_le_bytes());
            }
            sgd.extend_from_slice(&descs);
            sgd.extend_from_slice(&file.endpoints);
            sgd.extend_from_slice(&file.selectors);
            sgd.extend_from_slice(&file.tables);
            Ok(write_container(&Container {
                width,
                height,
                layers,
                faces,
                scheme: SCHEME_BASISLZ,
                dfd: &etc1s_dfd(srgb, alpha),
                sgd: &sgd,
                levels: &stored_levels,
            }))
        }
        other => Err(bad(format!("unknown Basis format {other}"))),
    }
}

fn le32(d: &[u8], at: usize) -> Option<usize> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

/// Rewrites the ETC1S (BasisLZ) data of a KTX2 file as a `.basis` file so that the `.basis`
/// transcoder can read it. `levels` are the file's level payloads, largest mip first.
pub(crate) fn basislz_to_basis(
    sgd: &[u8],
    levels: &[&[u8]],
    width: u32,
    height: u32,
    layers: u32,
    faces: u32,
    srgb: bool,
) -> Result<Vec<u8>, AssetError> {
    let images = layers * faces;
    let level_count = levels.len() as u32;
    let image_count = (images as usize)
        .checked_mul(level_count as usize)
        .filter(|&n| n <= 1 << 20)
        .ok_or_else(|| bad("too many images"))?;
    let header_err = || bad("truncated BasisLZ global data");
    let endpoint_count = sgd
        .get(0..2)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(header_err)?;
    let selector_count = sgd
        .get(2..4)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(header_err)?;
    let endpoints_len = le32(sgd, 4).ok_or_else(header_err)?;
    let selectors_len = le32(sgd, 8).ok_or_else(header_err)?;
    let tables_len = le32(sgd, 12).ok_or_else(header_err)?;
    if endpoint_count == 0
        || selector_count == 0
        || endpoints_len == 0
        || selectors_len == 0
        || tables_len == 0
    {
        return Err(bad("empty codebooks in BasisLZ global data"));
    }
    let descs_at = ETC1S_GLOBAL_HEADER;
    let blobs_at = descs_at + image_count * ETC1S_IMAGE_DESC;
    let take = |from: usize, len: usize| -> Result<Vec<u8>, AssetError> {
        from.checked_add(len)
            .and_then(|end| sgd.get(from..end))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| bad("BasisLZ global data is too short"))
    };
    let endpoints = take(blobs_at, endpoints_len)?;
    let selectors = take(blobs_at + endpoints_len, selectors_len)?;
    let tables = take(blobs_at + endpoints_len + selectors_len, tables_len)?;

    let mut slices = Vec::new();
    let mut has_alpha = false;
    for image in 0..images {
        for level in 0..level_count {
            let desc_at =
                descs_at + (level as usize * images as usize + image as usize) * ETC1S_IMAGE_DESC;
            let field = |i: usize| le32(sgd, desc_at + 4 * i).ok_or_else(header_err);
            let (rgb_at, rgb_len, alpha_at, alpha_len) =
                (field(1)?, field(2)?, field(3)?, field(4)?);
            let payload = levels[level as usize];
            let cut = |at: usize, len: usize| -> Result<Vec<u8>, AssetError> {
                at.checked_add(len)
                    .and_then(|end| payload.get(at..end))
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| bad("slice lies outside its mip level"))
            };
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            let mut push = |data: Vec<u8>, flags: u8| {
                slices.push(Slice {
                    image,
                    level,
                    flags,
                    width: w,
                    height: h,
                    blocks_x: w.div_ceil(4),
                    blocks_y: h.div_ceil(4),
                    data,
                });
            };
            push(cut(rgb_at, rgb_len)?, 0);
            if alpha_len > 0 {
                has_alpha = true;
                push(cut(alpha_at, alpha_len)?, SLICE_HAS_ALPHA);
            }
        }
    }
    let tex_type = if faces == 6 {
        TYPE_CUBEMAP_ARRAY
    } else if layers > 1 {
        TYPE_2D_ARRAY
    } else {
        TYPE_2D
    };
    Ok(BasisFile {
        format: FORMAT_ETC1S,
        flags: FLAG_ETC1S
            | if has_alpha { FLAG_HAS_ALPHA_SLICES } else { 0 }
            | if srgb { FLAG_SRGB } else { 0 },
        tex_type,
        images,
        endpoint_count: endpoint_count as u32,
        selector_count: selector_count as u32,
        endpoints,
        selectors,
        tables,
        slices,
    }
    .build())
}

/// Wraps raw UASTC levels (as stored in a KTX2 file, free of Zstandard) in a `.basis` file.
///
/// `levels[mip]` holds `layers * faces` sub-images of `ceil(w/4) * ceil(h/4)` 16-byte blocks.
pub(crate) fn uastc_to_basis(
    levels: &[Vec<u8>],
    width: u32,
    height: u32,
    layers: u32,
    faces: u32,
    alpha: bool,
    srgb: bool,
) -> Result<Vec<u8>, AssetError> {
    let images = layers * faces;
    let mut slices = Vec::new();
    for image in 0..images {
        for (level, data) in levels.iter().enumerate() {
            let level = level as u32;
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            let (blocks_x, blocks_y) = (w.div_ceil(4), h.div_ceil(4));
            let size = (blocks_x * blocks_y) as usize * 16;
            let part = data
                .get(image as usize * size..(image as usize + 1) * size)
                .ok_or_else(|| {
                    bad(format!(
                        "level {level} is too short for {images} sub-images"
                    ))
                })?;
            slices.push(Slice {
                image,
                level,
                flags: if alpha { SLICE_HAS_ALPHA } else { 0 },
                width: w,
                height: h,
                blocks_x,
                blocks_y,
                data: part.to_vec(),
            });
        }
    }
    let tex_type = if faces == 6 {
        TYPE_CUBEMAP_ARRAY
    } else if layers > 1 {
        TYPE_2D_ARRAY
    } else {
        TYPE_2D
    };
    Ok(BasisFile {
        format: FORMAT_UASTC,
        flags: if alpha { FLAG_HAS_ALPHA_SLICES } else { 0 } | if srgb { FLAG_SRGB } else { 0 },
        tex_type,
        images,
        endpoint_count: 0,
        selector_count: 0,
        endpoints: Vec::new(),
        selectors: Vec::new(),
        tables: Vec::new(),
        slices,
    }
    .build())
}

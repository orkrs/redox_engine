//! `.sdf` file format (GI-06): the baked distance-field bricks of one world cell.
//!
//! Layout (little endian): `b"RSDF"`, version `u32`, brick count `u32`, then per brick
//! `origin_ws` (3 × f32), `world_size` (3 × f32), the quantisation range `D` (f32) and
//! `64³` texels as `u16`; the last 4 bytes are the FNV-1a hash of everything before them.
//! A texel stores `round((d / D + 1) * 32767)` for `d` clamped to `[-D, D]`, where
//! `D = 0.5 * max(world_size)`.

use redox_math::Vec3;

use super::atlas::SdfBrick;
use super::bake::{DistanceField64, SDF_RESOLUTION};

/// File magic.
pub const MAGIC: [u8; 4] = *b"RSDF";
/// Current format version.
pub const VERSION: u32 = 1;
const TEXELS: usize = SDF_RESOLUTION * SDF_RESOLUTION * SDF_RESOLUTION;
const BRICK_HEADER: usize = 7 * 4;

/// Why a `.sdf` file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdfFormatError {
    /// Shorter than its header or brick list says.
    Truncated,
    /// Does not start with `RSDF`.
    BadMagic,
    /// Written by a newer format version.
    UnknownVersion(u32),
    /// The stored hash does not match the bytes.
    Corrupted,
}

impl std::fmt::Display for SdfFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "sdf file is truncated"),
            Self::BadMagic => write!(f, "not an RSDF file"),
            Self::UnknownVersion(v) => write!(f, "unknown sdf version {v}"),
            Self::Corrupted => write!(f, "sdf file is corrupted (hash mismatch)"),
        }
    }
}

impl std::error::Error for SdfFormatError {}

/// FNV-1a, 32 bit (the hash the `.rvg` pages use as well).
fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for &b in bytes {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Quantisation range of a brick: half its longest edge.
pub fn quantization_range(world_size: Vec3) -> f32 {
    0.5 * world_size.max_element().max(1e-6)
}

/// Quantises `field` to `u16` texels; returns them with the range `D` used.
pub fn quantize_field(field: &DistanceField64) -> (Vec<u16>, f32) {
    let range = quantization_range(field.world_size);
    let texels = field
        .data
        .iter()
        .map(|&d| {
            ((d / range).clamp(-1.0, 1.0) + 1.0)
                .mul_add(32767.0, 0.0)
                .round() as u16
        })
        .collect();
    (texels, range)
}

/// Inverse of [`quantize_field`]: distances in metres, exact to `range / 32767`.
pub fn dequantize(texels: &[u16], range: f32) -> Vec<f32> {
    texels
        .iter()
        .map(|&q| (f32::from(q) / 32767.0 - 1.0) * range)
        .collect()
}

/// Serialises `bricks`.
pub fn write_bricks(bricks: &[SdfBrick]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + bricks.len() * (BRICK_HEADER + TEXELS * 2) + 4);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(bricks.len() as u32).to_le_bytes());
    for brick in bricks {
        for v in brick.origin_ws.to_array() {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in brick.world_size.to_array() {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let (texels, range) = quantize_field(&brick.field);
        out.extend_from_slice(&range.to_le_bytes());
        for q in texels {
            out.extend_from_slice(&q.to_le_bytes());
        }
    }
    let hash = fnv1a32(&out);
    out.extend_from_slice(&hash.to_le_bytes());
    out
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, SdfFormatError> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(SdfFormatError::Truncated)
}

fn read_f32(bytes: &[u8], at: usize) -> Result<f32, SdfFormatError> {
    read_u32(bytes, at).map(f32::from_bits)
}

/// Parses a `.sdf` file.
///
/// # Errors
/// [`SdfFormatError`] for a truncated, foreign, newer or corrupted file; never panics.
pub fn read_bricks(bytes: &[u8]) -> Result<Vec<SdfBrick>, SdfFormatError> {
    if bytes.len() < 16 {
        return Err(SdfFormatError::Truncated);
    }
    if bytes[..4] != MAGIC {
        return Err(SdfFormatError::BadMagic);
    }
    let version = read_u32(bytes, 4)?;
    if version != VERSION {
        return Err(SdfFormatError::UnknownVersion(version));
    }
    let count = read_u32(bytes, 8)? as usize;
    let body = bytes.len() - 4;
    let expected = 12usize
        .checked_add(
            count
                .checked_mul(BRICK_HEADER + TEXELS * 2)
                .ok_or(SdfFormatError::Truncated)?,
        )
        .ok_or(SdfFormatError::Truncated)?;
    if body != expected {
        return Err(SdfFormatError::Truncated);
    }
    if read_u32(bytes, body)? != fnv1a32(&bytes[..body]) {
        return Err(SdfFormatError::Corrupted);
    }
    let mut at = 12;
    let mut bricks = Vec::with_capacity(count);
    for _ in 0..count {
        let origin_ws = Vec3::new(
            read_f32(bytes, at)?,
            read_f32(bytes, at + 4)?,
            read_f32(bytes, at + 8)?,
        );
        let world_size = Vec3::new(
            read_f32(bytes, at + 12)?,
            read_f32(bytes, at + 16)?,
            read_f32(bytes, at + 20)?,
        );
        let range = read_f32(bytes, at + 24)?;
        at += BRICK_HEADER;
        let texels: Vec<u16> = bytes[at..at + TEXELS * 2]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        at += TEXELS * 2;
        bricks.push(SdfBrick {
            field: DistanceField64 {
                data: dequantize(&texels, range),
                world_size,
            },
            origin_ws,
            world_size,
        });
    }
    Ok(bricks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brick(seed: f32) -> SdfBrick {
        let data = (0..TEXELS)
            .map(|i| ((i % 97) as f32 * 0.03 - 1.4) * seed)
            .collect();
        SdfBrick {
            field: DistanceField64 {
                data,
                world_size: Vec3::splat(4.0),
            },
            origin_ws: Vec3::new(1.0, 2.0, 3.0),
            world_size: Vec3::splat(4.0),
        }
    }

    #[test]
    fn a_brick_roundtrips_through_the_file_within_the_quantization_error() {
        let original = brick(1.0);
        let back = read_bricks(&write_bricks(std::slice::from_ref(&original))).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].origin_ws, original.origin_ws);
        assert_eq!(back[0].world_size, original.world_size);
        let limit = quantization_range(original.world_size) / 32767.0;
        for (a, b) in original.field.data.iter().zip(&back[0].field.data) {
            assert!((a - b).abs() <= limit * 1.01, "{a} vs {b}");
        }
    }

    #[test]
    fn a_corrupted_file_is_rejected() {
        let mut bytes = write_bricks(&[brick(1.0)]);
        bytes[100] ^= 0x10;
        assert_eq!(read_bricks(&bytes).err(), Some(SdfFormatError::Corrupted));
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let mut bytes = write_bricks(&[brick(1.0)]);
        bytes[4] = 9;
        assert_eq!(
            read_bricks(&bytes).err(),
            Some(SdfFormatError::UnknownVersion(9))
        );
    }

    #[test]
    fn an_empty_brick_list_roundtrips() {
        assert!(read_bricks(&write_bricks(&[])).unwrap().is_empty());
    }

    #[test]
    fn a_truncated_file_is_an_error_not_a_panic() {
        let bytes = write_bricks(&[brick(1.0)]);
        for cut in [0, 3, 11, 40, bytes.len() - 1] {
            assert!(read_bricks(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        assert_eq!(
            read_bricks(b"XXXXxxxxxxxxxxxxxxxx").err(),
            Some(SdfFormatError::BadMagic)
        );
    }

    #[test]
    fn bricks_loaded_from_bytes_produce_the_same_global_sdf_as_in_memory_bricks() {
        use crate::sdf::global::GlobalSdf;
        use redox_math::Aabb;
        let original = brick(1.0);
        let loaded = read_bricks(&write_bricks(std::slice::from_ref(&original))).unwrap();
        let bounds = Aabb {
            min: Vec3::new(-2.0, -1.0, 0.0),
            max: Vec3::new(4.0, 5.0, 6.0),
        };
        let dims = [16, 16, 16];
        let a = GlobalSdf::from_bricks(&[(original.clone(), [0.5; 4])], bounds, dims);
        let b = GlobalSdf::from_bricks(&[(loaded[0].clone(), [0.5; 4])], bounds, dims);
        let limit = quantization_range(original.world_size) / 32767.0 * 2.0;
        for (x, y) in a.distance.iter().zip(&b.distance) {
            assert!((x - y).abs() <= limit.max(1e-3), "{x} vs {y}");
        }
    }
}

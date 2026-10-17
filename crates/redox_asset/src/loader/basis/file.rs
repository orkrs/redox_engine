//! The `.basis` container: header and slice descriptors (see `basisu_file_headers.h`).
//!
//! Only what the KTX2 bridge needs: parsing a file into a plain description and building a
//! valid file back from codebooks and slice data, with correct CRC16 checksums.

use crate::error::AssetError;

pub(super) const HEADER_SIZE: usize = 77;
const SLICE_DESC_SIZE: usize = 23;
const SIGNATURE: u16 = (b'B' as u16) << 8 | b's' as u16;
const VERSION: u16 = 0x13;

pub(super) const FLAG_ETC1S: u16 = 1;
pub(super) const FLAG_HAS_ALPHA_SLICES: u16 = 4;
pub(super) const FLAG_SRGB: u16 = 16;
pub(super) const SLICE_HAS_ALPHA: u8 = 1;

pub(super) const FORMAT_ETC1S: u8 = 0;
pub(super) const FORMAT_UASTC: u8 = 1;
pub(super) const TYPE_2D: u8 = 0;
pub(super) const TYPE_2D_ARRAY: u8 = 1;
pub(super) const TYPE_CUBEMAP_ARRAY: u8 = 2;

fn bad(msg: impl Into<String>) -> AssetError {
    AssetError::LoadFailed(format!("basis: {}", msg.into()))
}

/// CRC16 as used by the Basis file format.
pub(super) fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = !0;
    for &byte in data {
        let q = u16::from(byte) ^ (crc >> 8);
        let k = (q >> 4) ^ q;
        crc = ((crc << 8) ^ k) ^ (k << 5) ^ (k << 12);
    }
    !crc
}

/// One compressed slice (a mip level of an image; ETC1S alpha lives in a second slice).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Slice {
    pub image: u32,
    pub level: u32,
    pub flags: u8,
    pub width: u32,
    pub height: u32,
    pub blocks_x: u32,
    pub blocks_y: u32,
    pub data: Vec<u8>,
}

/// Everything in a `.basis` file that matters for transcoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BasisFile {
    pub format: u8,
    pub flags: u16,
    pub tex_type: u8,
    pub images: u32,
    pub endpoint_count: u32,
    pub selector_count: u32,
    pub endpoints: Vec<u8>,
    pub selectors: Vec<u8>,
    pub tables: Vec<u8>,
    pub slices: Vec<Slice>,
}

fn u16_at(d: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([d[at], d[at + 1]]))
}

fn u24_at(d: &[u8], at: usize) -> usize {
    usize::from(d[at]) | usize::from(d[at + 1]) << 8 | usize::from(d[at + 2]) << 16
}

fn u32_at(d: &[u8], at: usize) -> usize {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]) as usize
}

fn range(data: &[u8], offset: usize, size: usize, what: &str) -> Result<Vec<u8>, AssetError> {
    offset
        .checked_add(size)
        .and_then(|end| data.get(offset..end))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| bad(format!("{what} lies outside the file")))
}

impl BasisFile {
    /// Parses a `.basis` file.
    pub fn parse(data: &[u8]) -> Result<Self, AssetError> {
        if data.len() < HEADER_SIZE {
            return Err(bad("file shorter than its header"));
        }
        if u16_at(data, 0) != usize::from(SIGNATURE) || u16_at(data, 2) != usize::from(VERSION) {
            return Err(bad("wrong signature or version"));
        }
        let total_slices = u24_at(data, 14);
        let images = u24_at(data, 17) as u32;
        let slice_desc_at = u32_at(data, 65);
        let mut slices = Vec::with_capacity(total_slices.min(4096));
        for i in 0..total_slices {
            let at = slice_desc_at
                .checked_add(i * SLICE_DESC_SIZE)
                .filter(|&at| at + SLICE_DESC_SIZE <= data.len())
                .ok_or_else(|| bad("slice descriptors lie outside the file"))?;
            let (offset, size) = (u32_at(data, at + 13), u32_at(data, at + 17));
            slices.push(Slice {
                image: u24_at(data, at) as u32,
                level: u32::from(data[at + 3]),
                flags: data[at + 4],
                width: u16_at(data, at + 5) as u32,
                height: u16_at(data, at + 7) as u32,
                blocks_x: u16_at(data, at + 9) as u32,
                blocks_y: u16_at(data, at + 11) as u32,
                data: range(data, offset, size, "slice data")?,
            });
        }
        Ok(Self {
            format: data[20],
            flags: u16_at(data, 21) as u16,
            tex_type: data[23],
            images,
            endpoint_count: u16_at(data, 39) as u32,
            selector_count: u16_at(data, 48) as u32,
            endpoints: range(
                data,
                u32_at(data, 41),
                u24_at(data, 45),
                "endpoint codebook",
            )?,
            selectors: range(
                data,
                u32_at(data, 50),
                u24_at(data, 54),
                "selector codebook",
            )?,
            tables: range(data, u32_at(data, 57), u32_at(data, 61), "Huffman tables")?,
            slices,
        })
    }

    /// Builds a valid `.basis` file (checksums included).
    pub fn build(&self) -> Vec<u8> {
        let descs_at = HEADER_SIZE;
        let endpoints_at = descs_at + self.slices.len() * SLICE_DESC_SIZE;
        let selectors_at = endpoints_at + self.endpoints.len();
        let tables_at = selectors_at + self.selectors.len();
        let mut slice_at = tables_at + self.tables.len();

        let mut body = Vec::new();
        for slice in &self.slices {
            let mut desc = Vec::with_capacity(SLICE_DESC_SIZE);
            desc.extend_from_slice(&slice.image.to_le_bytes()[..3]);
            desc.push(slice.level as u8);
            desc.push(slice.flags);
            for v in [slice.width, slice.height, slice.blocks_x, slice.blocks_y] {
                desc.extend_from_slice(&(v as u16).to_le_bytes());
            }
            desc.extend_from_slice(&(slice_at as u32).to_le_bytes());
            desc.extend_from_slice(&(slice.data.len() as u32).to_le_bytes());
            desc.extend_from_slice(&crc16(&slice.data).to_le_bytes());
            body.extend_from_slice(&desc);
            slice_at += slice.data.len();
        }
        body.extend_from_slice(&self.endpoints);
        body.extend_from_slice(&self.selectors);
        body.extend_from_slice(&self.tables);
        for slice in &self.slices {
            body.extend_from_slice(&slice.data);
        }

        let mut header = vec![0u8; HEADER_SIZE];
        let put = |h: &mut Vec<u8>, at: usize, v: u64, len: usize| {
            h[at..at + len].copy_from_slice(&v.to_le_bytes()[..len]);
        };
        put(&mut header, 0, u64::from(SIGNATURE), 2);
        put(&mut header, 2, u64::from(VERSION), 2);
        put(&mut header, 4, HEADER_SIZE as u64, 2);
        put(&mut header, 8, body.len() as u64, 4);
        put(&mut header, 12, u64::from(crc16(&body)), 2);
        put(&mut header, 14, self.slices.len() as u64, 3);
        put(&mut header, 17, u64::from(self.images), 3);
        header[20] = self.format;
        put(&mut header, 21, u64::from(self.flags), 2);
        header[23] = self.tex_type;
        put(&mut header, 39, u64::from(self.endpoint_count), 2);
        put(&mut header, 41, endpoints_at as u64, 4);
        put(&mut header, 45, self.endpoints.len() as u64, 3);
        put(&mut header, 48, u64::from(self.selector_count), 2);
        put(&mut header, 50, selectors_at as u64, 4);
        put(&mut header, 54, self.selectors.len() as u64, 3);
        put(&mut header, 57, tables_at as u64, 4);
        put(&mut header, 61, self.tables.len() as u64, 4);
        put(&mut header, 65, descs_at as u64, 4);
        let header_crc = crc16(&header[8..]);
        put(&mut header, 6, u64::from(header_crc), 2);

        header.extend_from_slice(&body);
        header
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ETC: &[u8] = include_bytes!("../../../tests/data/rust-logo-etc.basis");
    const UASTC: &[u8] = include_bytes!("../../../tests/data/rust-logo-uastc.basis");

    #[test]
    fn real_files_parse() {
        let etc = BasisFile::parse(ETC).unwrap();
        assert_eq!(
            (etc.format, etc.images, etc.slices.len()),
            (FORMAT_ETC1S, 1, 14)
        );
        assert_eq!(etc.flags & FLAG_ETC1S, FLAG_ETC1S);
        assert!(etc.endpoint_count > 0 && !etc.tables.is_empty());
        let uastc = BasisFile::parse(UASTC).unwrap();
        assert_eq!((uastc.format, uastc.slices.len()), (FORMAT_UASTC, 7));
        assert_eq!(uastc.slices[0].width, 64);
    }

    #[test]
    fn rebuilt_files_parse_back_equal_and_pass_the_library_checksums() {
        let transcoder = basis_universal::Transcoder::new();
        for original in [ETC, UASTC] {
            let parsed = BasisFile::parse(original).unwrap();
            let rebuilt = parsed.build();
            assert_eq!(BasisFile::parse(&rebuilt).unwrap(), parsed);
            assert!(transcoder.validate_header(&rebuilt));
            assert!(
                transcoder.validate_file_checksums(&rebuilt, true),
                "header and data CRC16 computed by us must satisfy the C++ library"
            );
        }
    }

    #[test]
    fn truncated_or_corrupt_files_error_out() {
        for cut in 0..ETC.len() {
            let _ = BasisFile::parse(&ETC[..cut]);
        }
        assert!(BasisFile::parse(b"junk").is_err());
        let mut wrong = ETC.to_vec();
        wrong[0] ^= 1;
        assert!(BasisFile::parse(&wrong).is_err());
    }
}

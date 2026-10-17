//! Compact binary world file (`world.bin`) for shipped games.
//!
//! The editable OFPA tree (`world.ron` + one file per actor) is slow to open for large worlds.
//! This format packs the manifest and every cell into one file with a cell index, so a
//! streamer opens it once and reads only the cells it needs.
//!
//! ```text
//! magic "RDXWLD1\0" | version u32 | index_len u32 | manifest_len u32
//! index (postcard, Vec<IndexEntry>) | manifest (postcard) | cell blobs back to back
//! ```
//!
//! Every blob (index, manifest, cell) is followed by an FNV-1a 64-bit checksum. Offsets in
//! the index are relative to the start of the cell data.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ofpa::{ActorRecord, WorldManifest, load_cell_actors, load_manifest, write_atomic};
use crate::{PartitionCell, SceneError, SceneResult};

const MAGIC: &[u8; 8] = b"RDXWLD1\0";
const VERSION: u32 = 1;
const HEADER_LEN: u64 = 8 + 4 + 4 + 4;
const CHECKSUM_LEN: u64 = 8;
/// Upper bound for the index and manifest blobs; protects against absurd allocations.
const MAX_META_LEN: u32 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct IndexEntry {
    cell: PartitionCell,
    offset: u64,
    len: u64,
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn blob(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + CHECKSUM_LEN as usize);
    out.extend_from_slice(bytes);
    out.extend_from_slice(&fnv1a(bytes).to_le_bytes());
    out
}

fn invalid(msg: &str) -> SceneError {
    SceneError::InvalidCell(format!("world.bin: {msg}"))
}

fn to_bytes<T: Serialize>(value: &T) -> SceneResult<Vec<u8>> {
    postcard::to_allocvec(value).map_err(|e| invalid(&format!("encode: {e}")))
}

fn from_checked<T: for<'de> Deserialize<'de>>(blob: &[u8]) -> SceneResult<T> {
    let split = blob
        .len()
        .checked_sub(CHECKSUM_LEN as usize)
        .ok_or_else(|| invalid("blob shorter than its checksum"))?;
    let (data, sum) = blob.split_at(split);
    if sum != fnv1a(data).to_le_bytes() {
        return Err(invalid("checksum mismatch"));
    }
    postcard::from_bytes(data).map_err(|e| invalid(&format!("decode: {e}")))
}

/// Serializes a whole world to bytes. `manifest.cells` is replaced by the given cells.
pub fn world_to_bytes(
    mut manifest: WorldManifest,
    cells: &[(PartitionCell, Vec<ActorRecord>)],
) -> SceneResult<Vec<u8>> {
    manifest.cells = cells.iter().map(|(c, _)| *c).collect();
    let mut data = Vec::new();
    let mut index = Vec::with_capacity(cells.len());
    for (cell, actors) in cells {
        let encoded = blob(&to_bytes(actors)?);
        index.push(IndexEntry {
            cell: *cell,
            offset: data.len() as u64,
            len: encoded.len() as u64,
        });
        data.extend_from_slice(&encoded);
    }
    let index = blob(&to_bytes(&index)?);
    let manifest = blob(&to_bytes(&manifest)?);

    let mut out =
        Vec::with_capacity(HEADER_LEN as usize + index.len() + manifest.len() + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(index.len() as u32).to_le_bytes());
    out.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
    out.extend_from_slice(&index);
    out.extend_from_slice(&manifest);
    out.extend_from_slice(&data);
    Ok(out)
}

/// Writes a world file atomically.
pub fn write_world_bin(
    path: impl AsRef<Path>,
    manifest: WorldManifest,
    cells: &[(PartitionCell, Vec<ActorRecord>)],
) -> SceneResult<()> {
    write_atomic(path, &world_to_bytes(manifest, cells)?)?;
    Ok(())
}

/// Converts an OFPA tree at `root` into a binary world file.
pub fn convert_ofpa_to_bin(root: impl AsRef<Path>, out: impl AsRef<Path>) -> SceneResult<()> {
    let root = root.as_ref();
    let manifest = load_manifest(root)?;
    let mut cells = Vec::with_capacity(manifest.cells.len());
    for cell in &manifest.cells {
        let mut actors = load_cell_actors(root, *cell)?;
        actors.sort_by(|a, b| a.id.cmp(&b.id)); // directory order is arbitrary
        cells.push((*cell, actors));
    }
    write_world_bin(out, manifest, &cells)
}

/// An opened binary world: manifest and index in memory, cells read on demand.
#[derive(Debug)]
pub struct BinWorld {
    file: File,
    manifest: WorldManifest,
    index: Vec<IndexEntry>,
    data_start: u64,
    data_len: u64,
}

impl BinWorld {
    /// Opens `path`, validating the header, index and manifest (cells are checked when read).
    pub fn open(path: impl AsRef<Path>) -> SceneResult<Self> {
        let mut file = File::open(path)?;
        let total = file.metadata()?.len();
        let mut header = [0u8; HEADER_LEN as usize];
        file.read_exact(&mut header)
            .map_err(|_| invalid("file shorter than the header"))?;
        if &header[..8] != MAGIC {
            return Err(invalid("bad magic"));
        }
        if u32::from_le_bytes(header[8..12].try_into().expect("4 bytes")) != VERSION {
            return Err(invalid("unsupported version"));
        }
        let index_len = u32::from_le_bytes(header[12..16].try_into().expect("4 bytes"));
        let manifest_len = u32::from_le_bytes(header[16..20].try_into().expect("4 bytes"));
        if index_len > MAX_META_LEN || manifest_len > MAX_META_LEN {
            return Err(invalid("implausible metadata size"));
        }
        let data_start = HEADER_LEN + u64::from(index_len) + u64::from(manifest_len);
        if data_start > total {
            return Err(invalid("metadata exceeds the file"));
        }
        let mut index_blob = vec![0u8; index_len as usize];
        file.read_exact(&mut index_blob)?;
        let mut manifest_blob = vec![0u8; manifest_len as usize];
        file.read_exact(&mut manifest_blob)?;
        let index: Vec<IndexEntry> = from_checked(&index_blob)?;
        let data_len = total - data_start;
        for entry in &index {
            if entry
                .offset
                .checked_add(entry.len)
                .is_none_or(|end| end > data_len)
            {
                return Err(invalid("cell range outside the file"));
            }
        }
        Ok(Self {
            file,
            manifest: from_checked(&manifest_blob)?,
            index,
            data_start,
            data_len,
        })
    }

    /// The world header.
    pub fn manifest(&self) -> &WorldManifest {
        &self.manifest
    }

    /// Cells stored in the file.
    pub fn cells(&self) -> impl Iterator<Item = PartitionCell> + '_ {
        self.index.iter().map(|e| e.cell)
    }

    /// Reads one cell's actors; an unknown cell yields none.
    pub fn load_cell(&mut self, cell: PartitionCell) -> SceneResult<Vec<ActorRecord>> {
        let Some(entry) = self.index.iter().find(|e| e.cell == cell).copied() else {
            return Ok(Vec::new());
        };
        debug_assert!(entry.offset + entry.len <= self.data_len);
        self.file
            .seek(SeekFrom::Start(self.data_start + entry.offset))?;
        let mut buf = vec![0u8; entry.len as usize];
        self.file.read_exact(&mut buf)?;
        from_checked(&buf)
    }
}

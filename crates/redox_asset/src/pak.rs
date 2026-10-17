//! `.pak` archives and the [`AssetSource`] abstraction over folders and archives.
//!
//! Layout (all integers little-endian):
//!
//! ```text
//! magic "RDXPAK1\0" | version u32 (2) | entry_count u32 | index_offset u64
//! ... file data (possibly compressed), back to back ...
//! index at index_offset, per entry:
//!     name_len u16 | name (UTF-8) | offset u64 | stored_size u64 | hash u64
//!     | raw_size u64 | codec u8 (0 = stored, 1 = LZ4 block)
//! ```
//!
//! Entries are written sorted by name, so packing the same input twice produces
//! byte-identical archives. Each entry carries a hash of its *uncompressed* content that is verified on read.
//! [`PakWriter::set_compression`] enables LZ4; an entry is stored compressed only when that
//! makes it smaller.
//! Names are relative, `/`-separated and must not contain `..`, empty parts, `\` or a
//! drive prefix, which makes path traversal through an archive impossible.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::guid::content_hash;

const MAGIC: &[u8; 8] = b"RDXPAK1\0";
const VERSION: u32 = 2;
const CODEC_STORED: u8 = 0;
const CODEC_LZ4: u8 = 1;
/// LZ4 cannot expand more than about 255x; larger claimed sizes are corrupt (and would
/// otherwise allow huge allocations from a tiny file).
const MAX_LZ4_RATIO: u64 = 256;
const HEADER_LEN: u64 = 8 + 4 + 4 + 8;

/// Errors from reading or writing archives.
#[derive(Debug, thiserror::Error)]
pub enum PakError {
    /// Underlying I/O failure.
    #[error("pak I/O error: {0}")]
    Io(#[from] io::Error),
    /// The file is not a valid archive (message explains why).
    #[error("corrupt pak: {0}")]
    Corrupt(String),
    /// The archive has no entry with this name.
    #[error("no such entry in pak: {0}")]
    NotFound(String),
    /// An entry name is not a safe relative path.
    #[error("invalid pak entry name: {0:?}")]
    InvalidName(String),
}

/// Whether `name` is a safe archive entry name.
pub fn is_valid_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= u16::MAX as usize
        && !name.contains('\\')
        && !name.contains(':')
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Collects files and writes an archive.
#[derive(Debug, Default)]
pub struct PakWriter {
    entries: BTreeMap<String, Vec<u8>>,
    /// Entries that must never be compressed (already compressed, or read in pieces).
    raw: std::collections::BTreeSet<String>,
    compress: bool,
}

impl PakWriter {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or disables LZ4 compression of entries (off by default).
    pub fn set_compression(&mut self, enabled: bool) {
        self.compress = enabled;
    }

    /// Adds an entry that is stored as is even when compression is on (for data that is
    /// already compressed or that is read in pieces, such as textures).
    pub fn add_uncompressed(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), PakError> {
        self.add(name, bytes)?;
        self.raw.insert(name.to_owned());
        Ok(())
    }

    /// Adds (or replaces) an entry.
    pub fn add(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), PakError> {
        if !is_valid_entry_name(name) {
            return Err(PakError::InvalidName(name.to_owned()));
        }
        self.entries.insert(name.to_owned(), bytes);
        Ok(())
    }

    /// Adds every file below `dir` (recursively) under its relative path.
    pub fn add_dir(&mut self, dir: &Path) -> Result<(), PakError> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    walk(&path, out)?;
                } else {
                    out.push(path);
                }
            }
            Ok(())
        }
        let mut files = Vec::new();
        walk(dir, &mut files)?;
        for file in files {
            let rel = file
                .strip_prefix(dir)
                .unwrap_or(&file)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            self.add(&rel, fs::read(&file)?)?;
        }
        Ok(())
    }

    /// Number of entries added so far.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing was added.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serializes the archive to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = Vec::new();
        let mut index = Vec::new();
        for (name, bytes) in &self.entries {
            let offset = HEADER_LEN + data.len() as u64;
            let packed = (self.compress && !self.raw.contains(name))
                .then(|| lz4_flex::block::compress(bytes))
                .filter(|c| c.len() < bytes.len());
            let (stored, codec) = match &packed {
                Some(c) => (c.as_slice(), CODEC_LZ4),
                None => (bytes.as_slice(), CODEC_STORED),
            };
            data.extend_from_slice(stored);
            index.extend_from_slice(&(name.len() as u16).to_le_bytes());
            index.extend_from_slice(name.as_bytes());
            index.extend_from_slice(&offset.to_le_bytes());
            index.extend_from_slice(&(stored.len() as u64).to_le_bytes());
            index.extend_from_slice(&content_hash(bytes).to_le_bytes());
            index.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            index.push(codec);
        }
        let index_offset = HEADER_LEN + data.len() as u64;
        let mut out = Vec::with_capacity(HEADER_LEN as usize + data.len() + index.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&index_offset.to_le_bytes());
        out.extend_from_slice(&data);
        out.extend_from_slice(&index);
        out
    }

    /// Writes the archive to `path`.
    pub fn write_to(&self, path: &Path) -> Result<(), PakError> {
        let mut file = fs::File::create(path)?;
        file.write_all(&self.to_bytes())?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    offset: u64,
    /// Bytes on disk.
    size: u64,
    /// Hash of the uncompressed content.
    hash: u64,
    raw_size: u64,
    codec: u8,
}

/// Random-access reader over an archive file.
#[derive(Debug)]
pub struct PakReader {
    file: Mutex<fs::File>,
    entries: BTreeMap<String, Entry>,
}

fn corrupt<T>(msg: &str) -> Result<T, PakError> {
    Err(PakError::Corrupt(msg.to_owned()))
}

impl PakReader {
    /// Opens an archive and validates its header and index (not the file contents; those are
    /// verified on read).
    pub fn open(path: &Path) -> Result<Self, PakError> {
        let mut file = fs::File::open(path)?;
        let total = file.metadata()?.len();
        if total < HEADER_LEN {
            return corrupt("file shorter than the header");
        }
        let mut header = [0u8; HEADER_LEN as usize];
        file.read_exact(&mut header)?;
        if &header[..8] != MAGIC {
            return corrupt("bad magic");
        }
        let version = u32::from_le_bytes(header[8..12].try_into().expect("4 bytes"));
        if version != VERSION {
            return corrupt("unsupported version");
        }
        let count = u32::from_le_bytes(header[12..16].try_into().expect("4 bytes"));
        let index_offset = u64::from_le_bytes(header[16..24].try_into().expect("8 bytes"));
        if index_offset < HEADER_LEN || index_offset > total {
            return corrupt("index offset out of range");
        }
        file.seek(SeekFrom::Start(index_offset))?;
        let mut index = Vec::new();
        file.read_to_end(&mut index)?;

        let mut entries = BTreeMap::new();
        let mut pos = 0usize;
        for _ in 0..count {
            let take = |pos: &mut usize, n: usize| -> Result<&[u8], PakError> {
                let end = pos.checked_add(n).filter(|&e| e <= index.len());
                let Some(end) = end else {
                    return corrupt("index truncated");
                };
                let slice = &index[*pos..end];
                *pos = end;
                Ok(slice)
            };
            let name_len =
                u16::from_le_bytes(take(&mut pos, 2)?.try_into().expect("2 bytes")) as usize;
            let name = std::str::from_utf8(take(&mut pos, name_len)?)
                .map_err(|_| PakError::Corrupt("entry name is not UTF-8".into()))?
                .to_owned();
            let offset = u64::from_le_bytes(take(&mut pos, 8)?.try_into().expect("8 bytes"));
            let size = u64::from_le_bytes(take(&mut pos, 8)?.try_into().expect("8 bytes"));
            let hash = u64::from_le_bytes(take(&mut pos, 8)?.try_into().expect("8 bytes"));
            let raw_size = u64::from_le_bytes(take(&mut pos, 8)?.try_into().expect("8 bytes"));
            let codec = take(&mut pos, 1)?[0];
            match codec {
                CODEC_STORED if raw_size != size => return corrupt("stored entry size mismatch"),
                CODEC_STORED => {}
                CODEC_LZ4 if raw_size > size.saturating_mul(MAX_LZ4_RATIO) => {
                    return corrupt("implausible uncompressed size");
                }
                CODEC_LZ4 => {}
                _ => return corrupt("unknown codec"),
            }
            if !is_valid_entry_name(&name) {
                return Err(PakError::InvalidName(name));
            }
            let end = offset.checked_add(size);
            if offset < HEADER_LEN || end.is_none_or(|e| e > index_offset) {
                return corrupt("entry data out of range");
            }
            entries.insert(
                name,
                Entry {
                    offset,
                    size,
                    hash,
                    raw_size,
                    codec,
                },
            );
        }
        Ok(Self {
            file: Mutex::new(file),
            entries,
        })
    }

    /// Entry names in sorted order.
    pub fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.entries.keys().map(String::as_str)
    }

    /// Whether the archive has the entry.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Reads an entry and verifies its hash.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, PakError> {
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| PakError::NotFound(name.to_owned()))?;
        let mut buf = vec![0u8; entry.size as usize];
        {
            let mut file = self
                .file
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            file.seek(SeekFrom::Start(entry.offset))?;
            file.read_exact(&mut buf)?;
        }
        if entry.codec == CODEC_LZ4 {
            buf = lz4_flex::block::decompress(&buf, entry.raw_size as usize)
                .map_err(|e| PakError::Corrupt(format!("lz4: {e}")))?;
        }
        if buf.len() as u64 != entry.raw_size || content_hash(&buf) != entry.hash {
            return corrupt("entry content does not match its hash");
        }
        Ok(buf)
    }

    /// Size of an entry's content in bytes.
    pub fn content_size(&self, name: &str) -> Option<u64> {
        self.entries.get(name).map(|e| e.raw_size)
    }

    /// Reads `len` bytes starting at `offset` of an entry's content.
    ///
    /// Entries stored without compression are read directly from the file; the content hash
    /// covers the whole entry, so it is **not** checked for such partial reads. Compressed
    /// entries are read and verified as a whole, then sliced.
    pub fn read_range(&self, name: &str, offset: u64, len: usize) -> Result<Vec<u8>, PakError> {
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| PakError::NotFound(name.to_owned()))?;
        let end = offset
            .checked_add(len as u64)
            .filter(|&end| end <= entry.raw_size)
            .ok_or_else(|| PakError::Corrupt("range outside the entry".into()))?;
        if entry.codec != CODEC_STORED {
            let all = self.read(name)?;
            return Ok(all[offset as usize..end as usize].to_vec());
        }
        let mut buf = vec![0u8; len];
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        file.seek(SeekFrom::Start(entry.offset + offset))?;
        file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// Where asset bytes come from: a folder during development, an archive in a shipped game.
pub trait AssetSource: Send + Sync {
    /// Reads the file at a relative `/`-separated path.
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
    /// Whether the file exists.
    fn exists(&self, path: &str) -> bool;

    /// Size of the file in bytes.
    fn size(&self, path: &str) -> io::Result<u64> {
        self.read(path).map(|bytes| bytes.len() as u64)
    }

    /// Reads `len` bytes starting at `offset`. Sources override this to avoid reading the whole
    /// file; the default reads everything and slices.
    fn read_range(&self, path: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let all = self.read(path)?;
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        start
            .checked_add(len)
            .and_then(|end| all.get(start..end))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "range outside the file"))
    }
}

/// Reads assets from a directory. Paths that would leave the directory are rejected.
#[derive(Debug, Clone)]
pub struct DirSource {
    root: PathBuf,
}

impl DirSource {
    /// Serves files below `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, path: &str) -> io::Result<PathBuf> {
        if !is_valid_entry_name(path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid asset path {path:?}"),
            ));
        }
        Ok(self.root.join(path))
    }
}

impl AssetSource for DirSource {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        fs::read(self.resolve(path)?)
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).is_ok_and(|p| p.is_file())
    }

    fn size(&self, path: &str) -> io::Result<u64> {
        fs::metadata(self.resolve(path)?).map(|m| m.len())
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let mut file = fs::File::open(self.resolve(path)?)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// Reads assets from a `.pak` archive.
#[derive(Debug)]
pub struct PakSource {
    reader: PakReader,
}

impl PakSource {
    /// Opens the archive at `path`.
    pub fn open(path: &Path) -> Result<Self, PakError> {
        Ok(Self {
            reader: PakReader::open(path)?,
        })
    }
}

impl AssetSource for PakSource {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.reader.read(path).map_err(|e| match e {
            PakError::Io(e) => e,
            PakError::NotFound(_) => io::Error::new(io::ErrorKind::NotFound, e),
            other => io::Error::new(io::ErrorKind::InvalidData, other),
        })
    }

    fn exists(&self, path: &str) -> bool {
        self.reader.contains(path)
    }

    fn size(&self, path: &str) -> io::Result<u64> {
        self.reader
            .content_size(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, PakError::NotFound(path.into())))
    }

    fn read_range(&self, path: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        self.reader
            .read_range(path, offset, len)
            .map_err(|e| match e {
                PakError::Io(e) => e,
                PakError::NotFound(_) => io::Error::new(io::ErrorKind::NotFound, e),
                other => io::Error::new(io::ErrorKind::InvalidData, other),
            })
    }
}

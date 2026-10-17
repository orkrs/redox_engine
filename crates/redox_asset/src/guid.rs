//! Stable asset identity: GUIDs stored in `.meta` sidecar files and a registry that maps
//! GUIDs to paths.
//!
//! Every asset file `a/b.png` gets a sidecar `a/b.png.meta` (RON) containing its
//! [`AssetGuid`]. Moving the pair keeps the GUID, so references by GUID survive renames
//! and reorganisation. Copying a file together with its sidecar would produce a duplicate
//! GUID; [`AssetRegistry::scan`] detects that and gives the later path a fresh GUID.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Extension of sidecar files (without the dot).
pub const META_EXTENSION: &str = "meta";

/// 128-bit asset identifier, written as 32 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetGuid(pub u128);

impl AssetGuid {
    /// Generates a random (version 4) GUID.
    pub fn new_random() -> Self {
        Self(uuid::Uuid::new_v4().as_u128())
    }
}

impl fmt::Display for AssetGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl fmt::Debug for AssetGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetGuid({self})")
    }
}

/// A GUID string was not 32 hexadecimal digits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid asset guid {0:?}: expected 32 hexadecimal digits")]
pub struct ParseGuidError(String);

impl FromStr for AssetGuid {
    type Err = ParseGuidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ParseGuidError(s.to_owned()));
        }
        u128::from_str_radix(s, 16)
            .map(Self)
            .map_err(|_| ParseGuidError(s.to_owned()))
    }
}

impl Serialize for AssetGuid {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AssetGuid {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Contents of a `.meta` sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetMeta {
    /// Identity of the asset.
    pub guid: AssetGuid,
    /// Version of the importer that produced derived data (bumped to force a re-cook).
    #[serde(default)]
    pub importer_version: u32,
    /// Hash of the source file at the time the sidecar was last updated.
    #[serde(default)]
    pub source_hash: u64,
    /// Importer settings as free-form text (for example RON).
    #[serde(default)]
    pub settings: String,
}

/// FNV-1a 64-bit hash, used for change detection (not for security).
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Errors of the GUID registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// A file-system operation failed.
    #[error("{path}: {source}")]
    Io {
        /// File involved.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// A sidecar could not be parsed.
    #[error("{path}: invalid .meta file: {message}")]
    BadMeta {
        /// Sidecar involved.
        path: PathBuf,
        /// Parser message.
        message: String,
    },
    /// The asset is not known to the registry.
    #[error("unknown asset {0}")]
    Unknown(String),
    /// The destination of a move already exists.
    #[error("{0} already exists")]
    Occupied(PathBuf),
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> RegistryError + '_ {
    move |source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn meta_path(asset: &Path) -> PathBuf {
    let mut name = asset.as_os_str().to_owned();
    name.push(".");
    name.push(META_EXTENSION);
    PathBuf::from(name)
}

fn write_meta(asset: &Path, meta: &AssetMeta) -> Result<(), RegistryError> {
    let path = meta_path(asset);
    let text = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::new()).map_err(|e| {
        RegistryError::BadMeta {
            path: path.clone(),
            message: e.to_string(),
        }
    })?;
    fs::write(&path, text).map_err(io_err(&path))
}

fn read_meta(asset: &Path) -> Result<Option<AssetMeta>, RegistryError> {
    let path = meta_path(asset);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(&path)(e)),
    };
    ron::from_str(&text)
        .map(Some)
        .map_err(|e| RegistryError::BadMeta {
            path,
            message: e.to_string(),
        })
}

/// What [`AssetRegistry::scan`] found and fixed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Assets that had no sidecar and received one.
    pub created: Vec<PathBuf>,
    /// Assets that shared a GUID with an earlier path and received a fresh one.
    pub regenerated: Vec<PathBuf>,
    /// Assets whose sidecar could not be read; they are left out of the registry.
    pub damaged: Vec<PathBuf>,
}

/// Maps GUIDs to asset paths (relative to the registry root, `/`-separated).
#[derive(Debug)]
pub struct AssetRegistry {
    root: PathBuf,
    by_guid: HashMap<AssetGuid, String>,
    by_path: BTreeMap<String, AssetGuid>,
}

impl AssetRegistry {
    /// Scans `root` recursively, creating missing sidecars and repairing duplicate GUIDs.
    pub fn scan(root: impl Into<PathBuf>) -> Result<(Self, ScanReport), RegistryError> {
        let root = root.into();
        let mut files = Vec::new();
        collect_files(&root, &mut files)?;
        files.sort();

        let mut registry = Self {
            root,
            by_guid: HashMap::new(),
            by_path: BTreeMap::new(),
        };
        let mut report = ScanReport::default();
        for file in files {
            let rel = registry.relative(&file);
            let meta = match read_meta(&file) {
                Ok(Some(meta)) => meta,
                Ok(None) => {
                    let meta = new_meta(&file)?;
                    write_meta(&file, &meta)?;
                    report.created.push(PathBuf::from(&rel));
                    meta
                }
                Err(RegistryError::BadMeta { .. }) => {
                    report.damaged.push(PathBuf::from(&rel));
                    continue;
                }
                Err(e) => return Err(e),
            };
            let mut meta = meta;
            if registry.by_guid.contains_key(&meta.guid) {
                meta.guid = registry.fresh_guid();
                write_meta(&file, &meta)?;
                report.regenerated.push(PathBuf::from(&rel));
            }
            registry.by_guid.insert(meta.guid, rel.clone());
            registry.by_path.insert(rel, meta.guid);
        }
        Ok((registry, report))
    }

    fn fresh_guid(&self) -> AssetGuid {
        loop {
            let guid = AssetGuid::new_random();
            if !self.by_guid.contains_key(&guid) {
                return guid;
            }
        }
    }

    fn relative(&self, file: &Path) -> String {
        file.strip_prefix(&self.root)
            .unwrap_or(file)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Root directory of the registry.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Number of registered assets.
    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    /// Whether no asset is registered.
    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }

    /// Path (relative to the root) of an asset.
    pub fn path_of(&self, guid: AssetGuid) -> Option<&str> {
        self.by_guid.get(&guid).map(String::as_str)
    }

    /// GUID of the asset at a relative path.
    pub fn guid_of(&self, path: &str) -> Option<AssetGuid> {
        self.by_path.get(path).copied()
    }

    /// Iterates over `(path, guid)` pairs in path order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, AssetGuid)> + '_ {
        self.by_path.iter().map(|(p, g)| (p.as_str(), *g))
    }

    /// Renames or moves an asset together with its sidecar; the GUID is preserved.
    pub fn move_asset(&mut self, from: &str, to: &str) -> Result<AssetGuid, RegistryError> {
        let guid = self
            .guid_of(from)
            .ok_or_else(|| RegistryError::Unknown(from.to_owned()))?;
        let src = self.root.join(from);
        let dst = self.root.join(to);
        if dst.exists() || meta_path(&dst).exists() {
            return Err(RegistryError::Occupied(dst));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        fs::rename(&src, &dst).map_err(io_err(&src))?;
        fs::rename(meta_path(&src), meta_path(&dst)).map_err(io_err(&meta_path(&src)))?;
        self.by_path.remove(from);
        self.by_path.insert(to.to_owned(), guid);
        self.by_guid.insert(guid, to.to_owned());
        Ok(guid)
    }

    /// Deletes an asset and its sidecar.
    pub fn remove_asset(&mut self, path: &str) -> Result<(), RegistryError> {
        let guid = self
            .guid_of(path)
            .ok_or_else(|| RegistryError::Unknown(path.to_owned()))?;
        let file = self.root.join(path);
        fs::remove_file(&file).map_err(io_err(&file))?;
        let meta = meta_path(&file);
        match fs::remove_file(&meta) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(&meta)(e)),
        }
        self.by_path.remove(path);
        self.by_guid.remove(&guid);
        Ok(())
    }

    /// Whether the asset's file changed since its sidecar recorded the hash.
    pub fn is_modified(&self, path: &str) -> Result<bool, RegistryError> {
        let file = self.root.join(path);
        let meta = read_meta(&file)?.ok_or_else(|| RegistryError::Unknown(path.to_owned()))?;
        let bytes = fs::read(&file).map_err(io_err(&file))?;
        Ok(content_hash(&bytes) != meta.source_hash)
    }

    /// Records the current file hash in the sidecar (call after (re)importing).
    pub fn mark_imported(&self, path: &str, importer_version: u32) -> Result<(), RegistryError> {
        let file = self.root.join(path);
        let mut meta = read_meta(&file)?.ok_or_else(|| RegistryError::Unknown(path.to_owned()))?;
        let bytes = fs::read(&file).map_err(io_err(&file))?;
        meta.source_hash = content_hash(&bytes);
        meta.importer_version = importer_version;
        write_meta(&file, &meta)
    }
}

fn new_meta(file: &Path) -> Result<AssetMeta, RegistryError> {
    let bytes = fs::read(file).map_err(io_err(file))?;
    Ok(AssetMeta {
        guid: AssetGuid::new_random(),
        importer_version: 0,
        source_hash: content_hash(&bytes),
        settings: String::new(),
    })
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), RegistryError> {
    for entry in fs::read_dir(dir).map_err(io_err(dir))? {
        let path = entry.map_err(io_err(dir))?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.extension().is_none_or(|e| e != META_EXTENSION) {
            out.push(path);
        }
    }
    Ok(())
}

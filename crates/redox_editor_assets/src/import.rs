//! Importer framework of the editor: the importer trait, the importer registry, and two
//! validating importers (glTF through the engine loader, and texture headers read by hand).
//! Real import and cooking come later (EDT-B4, EDT-C1). Importers only read their source file.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use redox_asset::AssetGuid;

/// Cancellation flag shared between the queue and a running import.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Creates a token that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation; the importer stops at its next check.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One import request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportJob {
    /// Absolute path of the source file.
    pub path: PathBuf,
    /// Identity of the asset being imported.
    pub guid: AssetGuid,
    /// Importer settings as free-form text.
    pub settings: String,
}

/// Facts and warnings gathered by an importer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Facts such as sizes and counts.
    pub info: Vec<String>,
    /// Problems that do not fail the import.
    pub warnings: Vec<String>,
}

/// Successful result of an import.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportOutput {
    /// What the importer found.
    pub report: ImportReport,
}

/// Failure of an import.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// The source could not be imported; the message says why.
    #[error("{0}")]
    Failed(String),
    /// The import was cancelled before it finished.
    #[error("import cancelled")]
    Cancelled,
}

/// An importer for one or more file extensions.
pub trait AssetImporter: Send + Sync {
    /// Human-readable name.
    fn name(&self) -> &str;
    /// Extensions handled, lowercase, without the dot.
    fn extensions(&self) -> &[&str];
    /// Version of the importer; a change forces a re-import of its assets.
    fn version(&self) -> u32;
    /// Imports one source file. Must not modify the source file.
    fn import(&self, job: &ImportJob, cancel: &CancelToken) -> Result<ImportOutput, ImportError>;
}

/// The set of importers known to the editor.
#[derive(Clone, Default)]
pub struct ImporterRegistry {
    importers: Vec<Arc<dyn AssetImporter>>,
}

impl ImporterRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a registry with the two validating importers.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(GltfValidateImporter));
        registry.register(Arc::new(TextureProbeImporter));
        registry
    }

    /// Adds an importer; for an extension claimed twice, the first registered one wins.
    pub fn register(&mut self, importer: Arc<dyn AssetImporter>) {
        self.importers.push(importer);
    }

    /// Number of registered importers.
    pub fn len(&self) -> usize {
        self.importers.len()
    }

    /// Whether no importer is registered.
    pub fn is_empty(&self) -> bool {
        self.importers.is_empty()
    }

    /// The importer for a file, chosen by its extension (case-insensitive).
    pub fn for_path(&self, path: &Path) -> Option<Arc<dyn AssetImporter>> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        self.importers
            .iter()
            .find(|importer| importer.extensions().iter().any(|e| *e == ext))
            .cloned()
    }
}

fn check_cancel(cancel: &CancelToken) -> Result<(), ImportError> {
    if cancel.is_cancelled() {
        Err(ImportError::Cancelled)
    } else {
        Ok(())
    }
}

/// Loads a glTF or GLB file with the engine loader and reports its contents.
pub struct GltfValidateImporter;

impl AssetImporter for GltfValidateImporter {
    fn name(&self) -> &str {
        "gltf-validate"
    }

    fn extensions(&self) -> &[&str] {
        &["glb", "gltf"]
    }

    fn version(&self) -> u32 {
        1
    }

    fn import(&self, job: &ImportJob, cancel: &CancelToken) -> Result<ImportOutput, ImportError> {
        check_cancel(cancel)?;
        let scene = redox_asset::load_scene_from_path(&job.path)
            .map_err(|e| ImportError::Failed(format!("glTF load failed: {e}")))?;
        check_cancel(cancel)?;
        let mut report = ImportReport::default();
        report.info.push(format!(
            "{} nodes, {} meshes, {} materials, {} textures",
            scene.nodes.len(),
            scene.meshes.len(),
            scene.materials.len(),
            scene.textures.len()
        ));
        if scene.meshes.is_empty() {
            report.warnings.push("file has no mesh primitives".into());
        }
        Ok(ImportOutput { report })
    }
}

/// Reads the size and format of a PNG, JPEG or KTX2 file from its header.
pub struct TextureProbeImporter;

impl AssetImporter for TextureProbeImporter {
    fn name(&self) -> &str {
        "texture-probe"
    }

    fn extensions(&self) -> &[&str] {
        &["png", "jpg", "jpeg", "ktx2"]
    }

    fn version(&self) -> u32 {
        1
    }

    fn import(&self, job: &ImportJob, cancel: &CancelToken) -> Result<ImportOutput, ImportError> {
        check_cancel(cancel)?;
        let bytes = fs::read(&job.path)
            .map_err(|e| ImportError::Failed(format!("cannot read source: {e}")))?;
        check_cancel(cancel)?;
        let header = probe_texture_header(&bytes).map_err(ImportError::Failed)?;
        let mut report = ImportReport::default();
        report.info.push(format!(
            "{}x{} {}",
            header.width, header.height, header.format
        ));
        Ok(ImportOutput { report })
    }
}

/// Size and format read from a texture header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureHeader {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Short description of the format.
    pub format: String,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const KTX2_IDENTIFIER: [u8; 12] = [
    0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];
const JPEG_EOI: u8 = 0xD9;
const JPEG_DHT: u8 = 0xC4;
const JPEG_JPG: u8 = 0xC8;
const JPEG_DAC: u8 = 0xCC;

/// Reads the header of a PNG, JPEG or KTX2 file without decoding pixels.
pub fn probe_texture_header(bytes: &[u8]) -> Result<TextureHeader, String> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        probe_png(bytes)
    } else if bytes.starts_with(&JPEG_SOI) {
        probe_jpeg(bytes)
    } else if bytes.starts_with(&KTX2_IDENTIFIER) {
        probe_ktx2(bytes)
    } else {
        Err("unknown texture format (expected PNG, JPEG or KTX2)".into())
    }
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn probe_png(bytes: &[u8]) -> Result<TextureHeader, String> {
    if bytes.len() < 26 || &bytes[12..16] != b"IHDR" {
        return Err("PNG without an IHDR chunk".into());
    }
    let width = be_u32(&bytes[16..20]);
    let height = be_u32(&bytes[20..24]);
    if width == 0 || height == 0 {
        return Err("PNG with zero size".into());
    }
    let depth = bytes[24];
    let channels = match bytes[25] {
        0 => "gray",
        2 => "RGB",
        3 => "indexed",
        4 => "gray+alpha",
        6 => "RGBA",
        _ => "unknown channels",
    };
    Ok(TextureHeader {
        width,
        height,
        format: format!("PNG {channels} {depth}-bit"),
    })
}

fn probe_jpeg(bytes: &[u8]) -> Result<TextureHeader, String> {
    let mut pos = JPEG_SOI.len();
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            return Err("corrupt JPEG marker".into());
        }
        let marker = bytes[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if marker == JPEG_EOI {
            break;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        let is_frame = (0xC0..=0xCF).contains(&marker)
            && marker != JPEG_DHT
            && marker != JPEG_JPG
            && marker != JPEG_DAC;
        if is_frame {
            if pos + 9 > bytes.len() {
                return Err("truncated JPEG frame header".into());
            }
            let height = u32::from(u16::from_be_bytes([bytes[pos + 5], bytes[pos + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[pos + 7], bytes[pos + 8]]));
            if width == 0 || height == 0 {
                return Err("JPEG with zero size".into());
            }
            let components = bytes.get(pos + 9).copied().unwrap_or(0);
            return Ok(TextureHeader {
                width,
                height,
                format: format!("JPEG {components} components"),
            });
        }
        if pos + 4 > bytes.len() {
            break;
        }
        let len = usize::from(u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]));
        if len < 2 {
            return Err("corrupt JPEG segment length".into());
        }
        pos += 2 + len;
    }
    Err("JPEG without a frame header".into())
}

fn probe_ktx2(bytes: &[u8]) -> Result<TextureHeader, String> {
    if bytes.len() < 48 {
        return Err("truncated KTX2 header".into());
    }
    let vk_format = le_u32(&bytes[12..16]);
    let width = le_u32(&bytes[20..24]);
    let height = le_u32(&bytes[24..28]).max(1);
    let layers = le_u32(&bytes[32..36]).max(1);
    let faces = le_u32(&bytes[36..40]);
    if width == 0 {
        return Err("KTX2 with zero width".into());
    }
    let shape = if faces == 6 { "cube" } else { "2D" };
    Ok(TextureHeader {
        width,
        height,
        format: format!("KTX2 vkFormat {vk_format}, {shape}, {layers} layer(s)"),
    })
}

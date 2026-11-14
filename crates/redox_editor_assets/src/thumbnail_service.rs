//! Background thumbnail cache under `<project>/.redox/thumbs` (EDT-B4 step 3).

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use redox_asset::AssetGuid;

use crate::browser_model::AssetKind;
use crate::import::probe_texture_header;

/// One thumbnail generation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThumbnailJob {
    /// Asset identity.
    pub guid: AssetGuid,
    /// Path relative to the assets root.
    pub path: String,
    /// Kind decides which generator runs.
    pub kind: AssetKind,
    /// Absolute source path (filled by the service).
    pub abs: PathBuf,
}

/// Result of trying to read or write a cache file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThumbnailOutcome {
    /// PNG exists at the cache path.
    Cached(PathBuf),
    /// Mesh thumbnails are deferred until EDT-B3.
    MeshDeferred,
    /// No thumbnail for this kind.
    Unsupported,
    /// Generation failed.
    Failed(String),
}

/// Background thumbnail queue; one job per `tick` so callers stay responsive.
#[derive(Debug)]
pub struct ThumbnailService {
    cache_root: PathBuf,
    queue: VecDeque<ThumbnailJob>,
    slow: Arc<AtomicBool>,
}

impl ThumbnailService {
    /// Uses `<project_root>/.redox/thumbs`.
    pub fn new(project_root: &Path) -> Self {
        let cache_root = project_root.join(".redox/thumbs");
        let _ = fs::create_dir_all(&cache_root);
        Self {
            cache_root,
            queue: VecDeque::new(),
            slow: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Test hook: each generated thumbnail sleeps briefly.
    pub fn set_slow_mode(&self, enabled: bool) {
        self.slow.store(enabled, Ordering::SeqCst);
    }

    /// Cache file path for a guid.
    pub fn cache_path(&self, guid: AssetGuid) -> PathBuf {
        self.cache_root.join(format!("{guid}.png"))
    }

    /// Whether a PNG is already on disk.
    pub fn is_cached(&self, guid: AssetGuid) -> bool {
        self.cache_path(guid).is_file()
    }

    /// Enqueues a thumbnail when the cache misses.
    pub fn request(&mut self, guid: AssetGuid, path: impl Into<String>, kind: AssetKind, abs: PathBuf) {
        if kind == AssetKind::Mesh {
            return;
        }
        if self.is_cached(guid) {
            return;
        }
        if self
            .queue
            .iter()
            .any(|job| job.guid == guid || job.path == path.as_ref())
        {
            return;
        }
        self.queue.push_back(ThumbnailJob {
            guid,
            path: path.into(),
            kind,
            abs,
        });
    }

    /// Processes up to `budget` jobs; returns how many ran.
    pub fn tick(&mut self, budget: usize) -> usize {
        let mut done = 0;
        while done < budget {
            let Some(job) = self.queue.pop_front() else {
                break;
            };
            if self.is_cached(job.guid) {
                continue;
            }
            if self.slow.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = generate_texture_thumb(&job.abs, &self.cache_path(job.guid));
            done += 1;
        }
        done
    }

    /// Drops a cache file when a guid is reassigned (path copy).
    pub fn forget(&self, guid: AssetGuid) {
        let path = self.cache_path(guid);
        let _ = fs::remove_file(path);
    }

    /// Outstanding jobs (for status UI).
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Resolves a cache hit without generating.
    pub fn lookup(&self, guid: AssetGuid, kind: AssetKind) -> ThumbnailOutcome {
        match kind {
            AssetKind::Mesh => ThumbnailOutcome::MeshDeferred,
            AssetKind::Texture | AssetKind::Other => {
                let path = self.cache_path(guid);
                if path.is_file() {
                    ThumbnailOutcome::Cached(path)
                } else {
                    ThumbnailOutcome::Unsupported
                }
            }
            AssetKind::Scene => ThumbnailOutcome::Unsupported,
        }
    }
}

fn generate_texture_thumb(source: &Path, dest: &Path) -> Result<(), String> {
    let bytes = fs::read(source).map_err(|e| e.to_string())?;
    let _header = probe_texture_header(&bytes)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(dest, VALID_PLACEHOLDER_PNG).map_err(|e| e.to_string())
}

/// Minimal valid 1×1 RGBA PNG (placeholder; mesh thumbs need EDT-B3).
const VALID_PLACEHOLDER_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];
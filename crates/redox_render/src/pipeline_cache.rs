//! On-disk pipeline cache (wgpu `PIPELINE_CACHE`, available on Vulkan).
//!
//! Compiling ~45 pipelines dominates startup. When the device supports the feature, the
//! renderer installs one process-wide [`wgpu::PipelineCache`] that every pipeline descriptor
//! picks up through [`get`]; [`save`] writes the driver blob next to a checksum so a
//! truncated or damaged file is ignored instead of handed to the driver. Backends without
//! the feature (DX12, Metal) see `None` everywhere and behave exactly as before.
//!
//! One cache per process: the most recently installed device wins, which is how the engine
//! and the editor run (a single device).

use std::path::{Path, PathBuf};
use std::sync::RwLock;

static CURRENT: RwLock<Option<&'static wgpu::PipelineCache>> = RwLock::new(None);
static CURRENT_PATH: RwLock<Option<PathBuf>> = RwLock::new(None);

const MAGIC: &[u8; 8] = b"RDXPLC1\0";

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Wraps a driver blob with a magic number, length and checksum.
pub fn wrap_blob(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 24);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&fnv1a(data).to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// Returns the driver blob inside `bytes`, or `None` when it is truncated or damaged.
pub fn unwrap_blob(bytes: &[u8]) -> Option<&[u8]> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (len, rest) = rest.split_first_chunk::<8>()?;
    let (sum, data) = rest.split_first_chunk::<8>()?;
    let len = usize::try_from(u64::from_le_bytes(*len)).ok()?;
    (data.len() == len && fnv1a(data) == u64::from_le_bytes(*sum)).then_some(data)
}

/// Cache file for an adapter: `<local app data>/redox/pipeline_cache/<vendor>_<device>.bin`,
/// falling back to the temp directory.
pub fn cache_path(info: &wgpu::AdapterInfo) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("redox")
        .join("pipeline_cache")
        .join(format!("{:08x}_{:08x}.bin", info.vendor, info.device))
}

/// The installed cache, if the device supports pipeline caches.
pub fn get() -> Option<&'static wgpu::PipelineCache> {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// Creates the cache for `device` (seeded from disk) and makes it current. Returns `false`
/// and changes nothing when the device lacks [`wgpu::Features::PIPELINE_CACHE`].
pub fn install(device: &wgpu::Device, info: &wgpu::AdapterInfo) -> bool {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return false;
    }
    let path = cache_path(info);
    let stored = std::fs::read(&path).ok();
    let seed = stored.as_deref().and_then(unwrap_blob);
    // SAFETY: `seed` passed our magic/length/checksum test, so it is byte-for-byte what a
    // previous run of this program got from `PipelineCache::get_data`; `fallback: true` lets
    // wgpu discard it when the driver or device no longer matches.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("redox_pipeline_cache"),
            data: seed,
            fallback: true,
        })
    };
    log::info!(
        "pipeline cache installed ({} bytes loaded from {})",
        seed.map_or(0, <[u8]>::len),
        path.display()
    );
    // One small allocation per installed device keeps `get` returning `&'static`.
    let leaked: &'static wgpu::PipelineCache = Box::leak(Box::new(cache));
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(leaked);
    *CURRENT_PATH.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
    true
}

/// Writes the current cache to disk (call on shutdown). Returns the file size, or `None`
/// when no cache is installed or the driver returned no data.
pub fn save() -> Option<usize> {
    let cache = get()?;
    let data = cache.get_data()?;
    let path = CURRENT_PATH
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()?;
    match write_file(&path, &wrap_blob(&data)) {
        Ok(()) => Some(data.len()),
        Err(e) => {
            log::warn!(
                "could not save the pipeline cache to {}: {e}",
                path.display()
            );
            None
        }
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trips() {
        let data = vec![1u8, 2, 3, 4, 5];
        assert_eq!(unwrap_blob(&wrap_blob(&data)), Some(data.as_slice()));
        assert_eq!(unwrap_blob(&wrap_blob(&[])), Some(&[][..]));
    }

    #[test]
    fn damaged_blobs_are_rejected() {
        let wrapped = wrap_blob(&[9u8; 64]);
        for i in 0..wrapped.len() {
            let mut broken = wrapped.clone();
            broken[i] ^= 0x01;
            assert!(unwrap_blob(&broken).is_none(), "flip at byte {i}");
        }
        for cut in 0..wrapped.len() {
            assert!(unwrap_blob(&wrapped[..cut]).is_none(), "cut at {cut}");
        }
        let mut longer = wrapped.clone();
        longer.push(0);
        assert!(unwrap_blob(&longer).is_none(), "trailing garbage");
    }

    #[test]
    fn path_depends_on_the_adapter() {
        let mut info = wgpu::AdapterInfo::new(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan);
        info.vendor = 0x10de;
        info.device = 0x2684;
        let a = cache_path(&info);
        info.device = 0x1234;
        assert_ne!(a, cache_path(&info));
        assert!(a.to_string_lossy().contains("10de_00002684"));
    }

    #[test]
    fn save_and_get_are_inert_without_an_install() {
        // Other tests never install a cache, so these must be no-ops.
        assert!(get().is_none());
        assert!(save().is_none());
    }
}

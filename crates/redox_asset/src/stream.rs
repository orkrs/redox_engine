//! Streaming textures: open the header first, then read mip levels on demand.
//!
//! [`StreamedTexture::open`] reads only the KTX2 header and level index. Mip levels are then
//! read one at a time ([`StreamedTexture::read_mip`]) with range reads from a folder or an
//! uncompressed `.pak` entry, so a 4096x4096 texture can show its small mips right away and
//! fetch the large ones later or never. Basis Universal files cannot be cut into independent
//! pieces (their codebooks are shared), so they are decoded once as a whole.
//!
//! [`AssetManager`] drives this: [`stream_texture`](AssetManager::stream_texture) loads the
//! header, [`request_mip`](AssetManager::request_mip) queues a level read at a priority, and
//! [`drain_mips`](AssetManager::drain_mips) hands finished levels to the renderer within a
//! per-frame byte budget.

use std::any::TypeId;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::AssetError;
use crate::handle::{AssetId, AssetStatus, Handle};
use crate::loader::{AssetLoader, Ktx2Loader, Ktx2Texture, LoadOptions, TextureFormat};
use crate::manager::{AssetManager, LoadComplete};
use crate::pak::{AssetSource, DirSource};
use crate::queue::Priority;
use crate::storage::AssetStorage;

/// Shape of a streamed texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    /// Width of mip 0.
    pub width: u32,
    /// Height of mip 0.
    pub height: u32,
    /// Pixel format.
    pub format: TextureFormat,
    /// Array layers.
    pub layers: u32,
    /// Faces per layer (1 or 6).
    pub faces: u32,
    /// Number of mip levels.
    pub mips: u32,
}

impl StreamInfo {
    /// Size of mip `mip` in pixels.
    pub fn mip_size(&self, mip: u32) -> (u32, u32) {
        ((self.width >> mip).max(1), (self.height >> mip).max(1))
    }

    /// Bytes of mip `mip` for all layers and faces.
    pub fn mip_bytes(&self, mip: u32) -> usize {
        let (w, h) = self.mip_size(mip);
        self.format.level_size(w, h) * (self.layers * self.faces) as usize
    }

    /// Bytes of the whole chain from `first_mip` down to the smallest level.
    pub fn bytes_from(&self, first_mip: u32) -> usize {
        (first_mip..self.mips).map(|m| self.mip_bytes(m)).sum()
    }

    /// Whether the texture is a cube map.
    pub fn is_cube(&self) -> bool {
        self.faces == 6
    }
}

struct LevelRange {
    offset: u64,
    length: u64,
    uncompressed: u64,
}

enum Backing {
    /// Levels are read with range reads (stored or Zstandard, one frame per level).
    Ranged {
        source: Arc<dyn AssetSource>,
        path: String,
        levels: Vec<LevelRange>,
        zstd: bool,
    },
    /// Everything was decoded when the file was opened.
    Decoded(Ktx2Texture),
}

struct Inner {
    info: StreamInfo,
    backing: Backing,
}

/// A texture whose mip levels are read on demand. Cheap to clone.
#[derive(Clone)]
pub struct StreamedTexture(Arc<Inner>);

impl std::fmt::Debug for StreamedTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("StreamedTexture")
            .field(&self.0.info)
            .finish()
    }
}

fn bad(msg: impl Into<String>) -> AssetError {
    AssetError::LoadFailed(format!("texture stream: {}", msg.into()))
}

fn io(path: &str, e: std::io::Error) -> AssetError {
    AssetError::Io {
        path: PathBuf::from(path),
        source: e,
    }
}

const MAX_MIPS: u32 = 16;

impl StreamedTexture {
    /// Opens `path` in `source`. Reads the header and level index only for KTX2 files with a
    /// plain format; anything else (Basis data, `.basis` files) is read and decoded whole.
    pub fn open(
        source: Arc<dyn AssetSource>,
        path: &str,
        options: LoadOptions,
    ) -> Result<Self, AssetError> {
        let head_len = ktx2::Header::LENGTH;
        let head = source.read_range(path, 0, head_len);
        if let Ok(head) = head
            && let Ok(head_bytes) = <&[u8; ktx2::Header::LENGTH]>::try_from(head.as_slice())
            && let Ok(header) = ktx2::Header::from_bytes(head_bytes)
            && let Some(vk) = header.format
            && let Some(format) = TextureFormat::from_vk(vk)
            && matches!(
                header.supercompression_scheme,
                None | Some(ktx2::SupercompressionScheme::Zstandard)
            )
            && header.pixel_depth <= 1
            && (header.face_count == 1 || header.face_count == 6)
            && (1..=MAX_MIPS).contains(&header.level_count.max(1))
        {
            return Self::open_ranged(source, path, &header, format);
        }
        // Not rangeable: decode the whole file once.
        let bytes = source.read(path).map_err(|e| io(path, e))?;
        let texture = Ktx2Loader::with_options(options).load(&bytes)?;
        Ok(Self(Arc::new(Inner {
            info: StreamInfo {
                width: texture.width,
                height: texture.height,
                format: texture.format,
                layers: texture.layers,
                faces: texture.faces,
                mips: texture.levels.len() as u32,
            },
            backing: Backing::Decoded(texture),
        })))
    }

    fn open_ranged(
        source: Arc<dyn AssetSource>,
        path: &str,
        header: &ktx2::Header,
        format: TextureFormat,
    ) -> Result<Self, AssetError> {
        let mips = header.level_count.max(1);
        let index_len = mips as usize * ktx2::LevelIndex::LENGTH;
        let index = source
            .read_range(path, ktx2::Header::LENGTH as u64, index_len)
            .map_err(|e| io(path, e))?;
        let file_size = source.size(path).map_err(|e| io(path, e))?;
        let zstd = header.supercompression_scheme == Some(ktx2::SupercompressionScheme::Zstandard);
        let mut levels = Vec::with_capacity(mips as usize);
        for chunk in index.chunks_exact(ktx2::LevelIndex::LENGTH) {
            let entry = ktx2::LevelIndex::from_bytes(chunk.try_into().expect("24 bytes"));
            let end = entry.byte_offset.checked_add(entry.byte_length);
            if end.is_none_or(|end| end > file_size) {
                return Err(bad("a mip level lies outside the file"));
            }
            levels.push(LevelRange {
                offset: entry.byte_offset,
                length: entry.byte_length,
                uncompressed: entry.uncompressed_byte_length,
            });
        }
        let info = StreamInfo {
            width: header.pixel_width,
            height: header.pixel_height.max(1),
            format,
            layers: header.layer_count.max(1),
            faces: header.face_count,
            mips,
        };
        if info.width > 16_384 || info.height > 16_384 || info.layers > 4096 {
            return Err(bad("texture is too large"));
        }
        Ok(Self(Arc::new(Inner {
            info,
            backing: Backing::Ranged {
                source,
                path: path.to_owned(),
                levels,
                zstd,
            },
        })))
    }

    /// Shape of the texture.
    pub fn info(&self) -> &StreamInfo {
        &self.0.info
    }

    /// The decoded bytes of mip `mip` (all layers and faces), reading just that level from the
    /// source when possible.
    pub fn read_mip(&self, mip: u32) -> Result<Vec<u8>, AssetError> {
        let info = &self.0.info;
        if mip >= info.mips {
            return Err(bad(format!("mip {mip} of {}", info.mips)));
        }
        let expected = info.mip_bytes(mip);
        match &self.0.backing {
            Backing::Decoded(texture) => Ok(texture.levels[mip as usize].clone()),
            Backing::Ranged {
                source,
                path,
                levels,
                zstd,
            } => {
                let range = &levels[mip as usize];
                let length = usize::try_from(range.length).map_err(|_| bad("level too large"))?;
                let bytes = source
                    .read_range(path, range.offset, length)
                    .map_err(|e| io(path, e))?;
                if *zstd {
                    if range.uncompressed != expected as u64 {
                        return Err(bad(format!(
                            "level {mip} declares {} decoded bytes, expected {expected}",
                            range.uncompressed
                        )));
                    }
                    crate::loader::zstd_decode_level(&bytes, expected, mip as usize)
                } else if bytes.len() == expected {
                    Ok(bytes)
                } else {
                    Err(bad(format!(
                        "level {mip} has {} bytes, expected {expected}",
                        bytes.len()
                    )))
                }
            }
        }
    }
}

/// A finished mip read, returned by [`AssetManager::drain_mips`].
pub struct MipReady {
    /// The streamed texture the data belongs to.
    pub texture: AssetId,
    /// Mip level (0 is the largest).
    pub mip: u32,
    /// The bytes of the level, or why reading failed.
    pub data: Result<Vec<u8>, String>,
}

/// Bookkeeping for mip requests (lives inside [`AssetManager`]).
#[derive(Default)]
pub(crate) struct MipState {
    pub requested: HashSet<(AssetId, u32)>,
    pub ready: VecDeque<MipReady>,
}

/// Queue key of a mip read: unique per (texture, mip) so single reads can be cancelled.
fn mip_key(id: AssetId, mip: u32) -> AssetId {
    AssetId(id.0.wrapping_shl(5) | u64::from(mip + 1))
}

type MipChannel = (flume::Sender<MipReady>, flume::Receiver<MipReady>);

impl AssetManager {
    /// Starts opening a streamed texture (header and level index only). The returned handle is
    /// `Loading` until the header has been read; then [`stream_info`](Self::stream_info) works.
    pub fn stream_texture(
        &mut self,
        path: &str,
        priority: Priority,
    ) -> Result<Handle<StreamedTexture>, AssetError> {
        let type_id = TypeId::of::<StreamedTexture>();
        let full = self.base_path.join(path);
        if let Some(&(tid, id)) = self.path_to_handle.get(&full)
            && tid == type_id
        {
            *self.refs.entry(id).or_insert(0) += 1;
            return Ok(Handle::new(id));
        }
        self.register_stream_type();
        let id = AssetId::next();
        self.storage_mut::<StreamedTexture>()
            .mark_loading(Handle::new(id));
        self.path_to_handle.insert(full.clone(), (type_id, id));
        self.track(id, type_id);

        let source: Arc<dyn AssetSource> = self
            .source
            .clone()
            .unwrap_or_else(|| Arc::new(DirSource::new(&self.base_path)));
        let options = self.stream_options;
        let rel = path.to_owned();
        let tx = self.complete_tx.clone();
        self.push_job(priority, id, move || {
            let result = StreamedTexture::open(source, &rel, options)
                .map(|t| Box::new(t) as Box<dyn std::any::Any + Send + Sync>)
                .map_err(|e| e.to_string());
            let _ = tx.send(LoadComplete {
                type_id,
                id,
                path: full,
                is_reload: false,
                result,
            });
        });
        Ok(Handle::new(id))
    }

    /// Options used when a streamed file has to be decoded whole (Basis data).
    pub fn set_stream_options(&mut self, options: LoadOptions) {
        self.stream_options = options;
    }

    /// Shape of a streamed texture whose header has been read.
    pub fn stream_info(&self, handle: Handle<StreamedTexture>) -> Option<StreamInfo> {
        self.get(handle).map(|t| *t.info())
    }

    /// Queues the read of mip `mip`. Returns `false` when the texture is not ready, the mip does
    /// not exist, or that mip is already requested (use [`set_mip_priority`](Self::set_mip_priority)).
    pub fn request_mip(
        &mut self,
        handle: Handle<StreamedTexture>,
        mip: u32,
        priority: Priority,
    ) -> bool {
        let Some(texture) = self.get(handle).cloned() else {
            return false;
        };
        if mip >= texture.info().mips || !self.mips.requested.insert((handle.id, mip)) {
            return false;
        }
        let id = handle.id;
        let tx = self.mip_channel().0.clone();
        self.push_job(priority, mip_key(id, mip), move || {
            let data = texture.read_mip(mip).map_err(|e| e.to_string());
            let _ = tx.send(MipReady {
                texture: id,
                mip,
                data,
            });
        });
        true
    }

    /// Changes the priority of a mip read that has not started. Returns whether it was waiting.
    pub fn set_mip_priority(
        &mut self,
        handle: Handle<StreamedTexture>,
        mip: u32,
        priority: Priority,
    ) -> bool {
        self.queue.set_priority(mip_key(handle.id, mip), priority)
    }

    /// Drops a requested mip read that has not started. Returns whether one was removed.
    pub fn cancel_mip(&mut self, handle: Handle<StreamedTexture>, mip: u32) -> bool {
        let removed = self.queue.cancel(mip_key(handle.id, mip)) > 0;
        if removed {
            self.mips.requested.remove(&(handle.id, mip));
        }
        removed
    }

    /// Mip reads waiting for a worker or for [`drain_mips`](Self::drain_mips).
    pub fn pending_mips(&self) -> usize {
        self.mips.requested.len()
    }

    /// Finished mip reads for textures that are still alive. Returns the oldest ones whose total
    /// size fits `max_bytes` (at least one if any is ready, so a big level cannot starve), and
    /// keeps the rest for the next frame.
    pub fn drain_mips(&mut self, max_bytes: usize) -> Vec<MipReady> {
        while let Ok(ready) = self.mip_channel().1.try_recv() {
            self.mips.requested.remove(&(ready.texture, ready.mip));
            if self.kinds.contains_key(&ready.texture) {
                self.mips.ready.push_back(ready);
            }
        }
        let mut out = Vec::new();
        let mut used = 0usize;
        while let Some(front) = self.mips.ready.front() {
            let size = front.data.as_ref().map_or(0, Vec::len);
            if !out.is_empty() && used + size > max_bytes {
                break;
            }
            used += size;
            out.extend(self.mips.ready.pop_front());
        }
        out
    }

    /// Forgets queued and finished mip reads of an unloaded texture.
    pub(crate) fn drop_mips_of(&mut self, id: AssetId) {
        let pending: Vec<_> = self
            .mips
            .requested
            .iter()
            .filter(|(t, _)| *t == id)
            .copied()
            .collect();
        for (texture, mip) in pending {
            self.queue.cancel(mip_key(texture, mip));
            self.mips.requested.remove(&(texture, mip));
        }
        self.mips.ready.retain(|m| m.texture != id);
    }

    fn mip_channel(&mut self) -> &MipChannel {
        self.mip_channel.get_or_insert_with(flume::unbounded)
    }

    /// Storage plumbing for [`StreamedTexture`] (it has no byte loader of its own).
    fn register_stream_type(&mut self) {
        let type_id = TypeId::of::<StreamedTexture>();
        self.inserters.entry(type_id).or_insert(|any, id, boxed| {
            if let Some(s) = any.downcast_mut::<AssetStorage<StreamedTexture>>()
                && let Ok(asset) = boxed.downcast::<StreamedTexture>()
            {
                s.insert(Handle::new(id), *asset);
            }
        });
        self.failed_markers.entry(type_id).or_insert(|any, id| {
            if let Some(s) = any.downcast_mut::<AssetStorage<StreamedTexture>>() {
                s.mark_failed(Handle::new(id));
            }
        });
        self.register_type_ops::<StreamedTexture>();
    }

    /// Whether the header of a streamed texture has been read.
    pub fn stream_status(&self, handle: Handle<StreamedTexture>) -> AssetStatus {
        self.status(handle)
    }
}

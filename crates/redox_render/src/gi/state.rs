//! Renderer-side GI state: the global SDF volume and its background rebuilds.
//!
//! A rebuild runs on a worker thread and is uploaded on the render thread once it finishes.
//! Until then the previous volume stays in use.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::card_atlas::{CARD_ATLAS_SIZE, CardAtlas};
use super::card_capture::ObjectCards;
use crate::sdf::global::{GLOBAL_SDF_DIMS, GlobalSdf};
use crate::sdf::global_gpu::GlobalSdfGpu;

/// Owns the GPU global SDF and the pending background rebuild, if any.
pub struct GiState {
    /// The volume currently used by GI.
    pub global_sdf: GlobalSdfGpu,
    /// Captured surface cards of the static objects (GI-02); `None` until `set_cards` is called.
    pub cards: Option<CardAtlas>,
    // A mutex only to make the renderer `Sync`; `Receiver` is not.
    pending: Option<Mutex<Receiver<GlobalSdf>>>,
}

impl GiState {
    /// State with an empty volume (every voxel far away).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            global_sdf: GlobalSdfGpu::new(device, queue, GLOBAL_SDF_DIMS),
            cards: None,
            pending: None,
        }
    }

    /// Starts `build` on a worker thread. A rebuild already in flight is abandoned: its result
    /// is dropped when it arrives, because the newer request supersedes it.
    pub fn spawn_rebuild(&mut self, build: impl FnOnce() -> GlobalSdf + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // The receiver is gone only when a newer rebuild replaced this one.
            let _ = tx.send(build());
        });
        self.pending = Some(Mutex::new(rx));
    }

    /// Uploads a finished rebuild, if one is ready. Call once per frame on the render thread.
    pub fn poll(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(rx) = self.pending.as_ref() else {
            return;
        };
        let received = rx
            .lock()
            .map_or(Err(TryRecvError::Disconnected), |rx| rx.try_recv());
        match received {
            Ok(sdf) => {
                self.global_sdf.upload(device, queue, &sdf);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending = None,
        }
    }

    /// Replaces the surface cards with `objects` (captured faces and their object ids). The
    /// atlas is created on first use at [`CARD_ATLAS_SIZE`]. Object ids index the trace's table.
    pub fn set_cards(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        objects: &[(ObjectCards, u32)],
    ) {
        let atlas = self
            .cards
            .get_or_insert_with(|| CardAtlas::new(device, CARD_ATLAS_SIZE));
        atlas.set_objects(device, queue, objects);
    }

    /// True while a rebuild is running or waiting for upload.
    pub fn rebuild_pending(&self) -> bool {
        self.pending.is_some()
    }
}

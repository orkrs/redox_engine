//! Renderer-side GI state: the global SDF volume and its background rebuilds.
//!
//! A rebuild runs on a worker thread and is uploaded on the render thread once it finishes.
//! Until then the previous volume stays in use.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::sdf::global::{GLOBAL_SDF_DIMS, GlobalSdf};
use crate::sdf::global_gpu::GlobalSdfGpu;

/// Owns the GPU global SDF and the pending background rebuild, if any.
pub struct GiState {
    /// The volume currently used by GI.
    pub global_sdf: GlobalSdfGpu,
    pending: Option<Receiver<GlobalSdf>>,
}

impl GiState {
    /// State with an empty volume (every voxel far away).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            global_sdf: GlobalSdfGpu::new(device, queue, GLOBAL_SDF_DIMS),
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
        self.pending = Some(rx);
    }

    /// Uploads a finished rebuild, if one is ready. Call once per frame on the render thread.
    pub fn poll(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(rx) = self.pending.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(sdf) => {
                self.global_sdf.upload(device, queue, &sdf);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending = None,
        }
    }

    /// True while a rebuild is running or waiting for upload.
    pub fn rebuild_pending(&self) -> bool {
        self.pending.is_some()
    }
}

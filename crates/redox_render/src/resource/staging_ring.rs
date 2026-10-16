//! Ring-buffered staging allocator for per-frame GPU uploads.
//!
//! Writes are staged into an aligned CPU scratch, then flushed with
//! `queue.write_buffer` (wgpu 0.20 `Buffer` is not `Clone`, so a GPU-side
//! staging copy chain stores destinations at flush via caller-provided refs).
//! Dynamic UBO/SSBO bind offsets must respect WebGPU alignment (default 256).

use std::collections::VecDeque;

/// Number of virtual frames the ring keeps alive before reclaiming space.
pub const FRAMES_IN_FLIGHT: usize = 2;

/// Default ring capacity (64 MiB).
pub const DEFAULT_RING_CAPACITY: u64 = 64 * 1024 * 1024;

/// A pending host→device upload recorded against the CPU scratch.
struct PendingWrite {
    src_offset: u64,
    size: u64,
    /// Index into the flush-time destination table supplied by the caller.
    dst_index: u32,
    dst_offset: u64,
}

/// Allocation result: byte range inside the staging buffer.
pub struct StagingAlloc<'a> {
    pub offset: u64,
    pub data: &'a mut [u8],
}

/// Ring staging buffer with per-frame watermarks.
pub struct StagingRing {
    capacity: u64,
    /// Next write cursor (bytes).
    head: u64,
    /// Alignments snapped from device limits (portable default 256).
    pub uniform_align: u64,
    pub storage_align: u64,
    /// Watermark at end of each completed frame (for reclaim).
    frame_ends: VecDeque<u64>,
    pending: Vec<PendingWrite>,
    /// CPU-side scratch for the current frame's uploads.
    cpu_scratch: Vec<u8>,
    scratch_used: usize,
}

impl StagingRing {
    pub fn new(device: &wgpu::Device, capacity: u64) -> Self {
        let limits = device.limits();
        let uniform_align = limits.min_uniform_buffer_offset_alignment as u64;
        let storage_align = limits.min_storage_buffer_offset_alignment as u64;
        Self {
            capacity,
            head: 0,
            uniform_align,
            storage_align,
            frame_ends: VecDeque::with_capacity(FRAMES_IN_FLIGHT),
            pending: Vec::with_capacity(64),
            cpu_scratch: vec![0u8; capacity as usize],
            scratch_used: 0,
        }
    }

    /// Align required for dynamic uniform / storage bind offsets.
    pub fn dynamic_align(&self) -> u64 {
        self.uniform_align.max(self.storage_align).max(256)
    }

    #[inline]
    fn align_up(value: u64, align: u64) -> u64 {
        debug_assert!(align.is_power_of_two());
        (value + align - 1) & !(align - 1)
    }

    fn alloc_offset(&mut self, size: u64, align: u64, dynamic: bool) -> Option<u64> {
        let align = if dynamic {
            align.max(self.dynamic_align())
        } else {
            align.max(4)
        };
        let offset = Self::align_up(self.head, align);
        let end = offset.checked_add(size)?;
        if end > self.capacity {
            self.head = 0;
            let offset = Self::align_up(0, align);
            let end = offset.checked_add(size)?;
            if end > self.capacity {
                return None;
            }
            self.head = end;
            return Some(offset);
        }
        self.head = end;
        Some(offset)
    }

    /// Reserve `size` bytes. Returns a mutable slice into the CPU scratch.
    pub fn alloc(&mut self, size: u64, align: u64, dynamic: bool) -> Option<StagingAlloc<'_>> {
        let offset = self.alloc_offset(size, align, dynamic)?;
        let start = offset as usize;
        let end = start + size as usize;
        self.scratch_used = self.scratch_used.max(end);
        Some(StagingAlloc {
            offset,
            data: &mut self.cpu_scratch[start..end],
        })
    }

    /// Copy `bytes` into the scratch and record a write to destination slot
    /// `dst_index` (index into the slice passed to [`Self::flush`]).
    pub fn write_slot(
        &mut self,
        dst_index: u32,
        dst_offset: u64,
        bytes: &[u8],
        dynamic: bool,
    ) -> bool {
        let size = bytes.len() as u64;
        let align = if dynamic { self.dynamic_align() } else { 4 };
        let Some(offset) = self.alloc_offset(size, align, dynamic) else {
            return false;
        };
        let start = offset as usize;
        let end = start + bytes.len();
        self.cpu_scratch[start..end].copy_from_slice(bytes);
        self.scratch_used = self.scratch_used.max(end);
        self.pending.push(PendingWrite {
            src_offset: offset,
            size,
            dst_index,
            dst_offset,
        });
        true
    }

    /// Flush all pending writes into `dests[dst_index]` via `queue.write_buffer`.
    pub fn flush(&mut self, queue: &wgpu::Queue, dests: &[&wgpu::Buffer]) {
        for w in self.pending.drain(..) {
            let Some(dst) = dests.get(w.dst_index as usize) else {
                continue;
            };
            let start = w.src_offset as usize;
            let end = start + w.size as usize;
            queue.write_buffer(dst, w.dst_offset, &self.cpu_scratch[start..end]);
        }
        self.scratch_used = 0;
    }

    /// Immediate upload (bypass pending queue) with ring alignment accounting.
    pub fn write_immediate(
        &mut self,
        queue: &wgpu::Queue,
        dst: &wgpu::Buffer,
        dst_offset: u64,
        bytes: &[u8],
        dynamic: bool,
    ) -> bool {
        let size = bytes.len() as u64;
        let align = if dynamic { self.dynamic_align() } else { 4 };
        if self.alloc_offset(size, align, dynamic).is_none() {
            // Ring full — still write directly so callers never fail silently.
            queue.write_buffer(dst, dst_offset, bytes);
            return true;
        }
        queue.write_buffer(dst, dst_offset, bytes);
        true
    }

    /// Mark end of frame; after `FRAMES_IN_FLIGHT` frames, reclaim prefix.
    pub fn end_frame(&mut self) {
        self.frame_ends.push_back(self.head);
        while self.frame_ends.len() > FRAMES_IN_FLIGHT {
            let _ = self.frame_ends.pop_front();
        }
        self.scratch_used = 0;
        self.pending.clear();
    }
}

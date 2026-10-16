//! Lightweight GPU timing: timestamp marks between passes, read back one frame later.
//!
//! Call [`GpuProfiler::mark`] after a stretch of GPU work; the time since the previous mark is
//! attributed to that label. Needs `TIMESTAMP_QUERY` and `TIMESTAMP_QUERY_INSIDE_ENCODERS`;
//! without them every call is a no-op and [`GpuProfiler::timings`] stays empty.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Features the profiler needs (request them when creating the device if the adapter has them).
pub const PROFILER_FEATURES: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// Marks per frame (including the frame-start mark).
const MAX_MARKS: u32 = 64;
const TICK_BYTES: u64 = 8;

/// One measured stretch of GPU work.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuTiming {
    pub label: &'static str,
    pub ms: f32,
}

enum Readback {
    Idle,
    /// Results were copied into the readback buffer during this frame's submission.
    Copied,
    /// `map_async` is in flight; the flag flips when the data can be read.
    Mapping(Arc<AtomicBool>),
}

pub struct GpuProfiler {
    query_set: Option<wgpu::QuerySet>,
    resolve: Option<wgpu::Buffer>,
    readback: Option<wgpu::Buffer>,
    period_ns: f32,
    labels: Vec<&'static str>,
    marks: u32,
    state: Readback,
    /// Labels of the frame whose results are being read back.
    in_flight: Vec<&'static str>,
    timings: Vec<GpuTiming>,
}

impl GpuProfiler {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let enabled = device.features().contains(PROFILER_FEATURES);
        let buffers = enabled.then(|| {
            let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu_profiler"),
                ty: wgpu::QueryType::Timestamp,
                count: MAX_MARKS,
            });
            let size = u64::from(MAX_MARKS) * TICK_BYTES;
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu_profiler_resolve"),
                size,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu_profiler_readback"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (query_set, resolve, readback)
        });
        let (query_set, resolve, readback) = match buffers {
            Some((q, r, b)) => (Some(q), Some(r), Some(b)),
            None => (None, None, None),
        };
        Self {
            query_set,
            resolve,
            readback,
            period_ns: queue.get_timestamp_period(),
            labels: Vec::new(),
            marks: 0,
            state: Readback::Idle,
            in_flight: Vec::new(),
            timings: Vec::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.query_set.is_some()
    }

    /// Latest completed measurement (one to a few frames old).
    pub fn timings(&self) -> &[GpuTiming] {
        &self.timings
    }

    /// One-line summary such as `forward 1.20 ms | gtao 0.80 ms | total 6.1 ms`.
    pub fn report(&self) -> String {
        if self.timings.is_empty() {
            return "gpu timings unavailable".to_string();
        }
        let mut out: Vec<String> = self
            .timings
            .iter()
            .filter(|t| t.ms >= 0.01)
            .map(|t| format!("{} {:.2}", t.label, t.ms))
            .collect();
        out.push(format!("total {:.2} ms", self.total_ms()));
        out.join(" | ")
    }

    /// Sum of the latest timings in milliseconds.
    pub fn total_ms(&self) -> f32 {
        self.timings.iter().map(|t| t.ms).sum()
    }

    /// Starts a frame: harvests finished readbacks and resets the mark list.
    pub fn begin_frame(&mut self, device: &wgpu::Device) {
        if !self.is_enabled() {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if let Readback::Mapping(done) = &self.state
            && done.load(Ordering::Acquire)
        {
            self.harvest();
        }
        self.labels.clear();
        self.marks = 0;
    }

    /// Writes a timestamp. `label` names the GPU work since the previous mark; the very first
    /// mark of a frame only sets the start time (its label is ignored).
    pub fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        let Some(query_set) = &self.query_set else {
            return;
        };
        if self.marks >= MAX_MARKS {
            return;
        }
        encoder.write_timestamp(query_set, self.marks);
        if self.marks > 0 {
            self.labels.push(label);
        }
        self.marks += 1;
    }

    /// Records the resolve + copy for this frame (only when the readback buffer is free).
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(query_set), Some(resolve), Some(readback)) =
            (&self.query_set, &self.resolve, &self.readback)
        else {
            return;
        };
        if !matches!(self.state, Readback::Idle) || self.marks < 2 {
            return;
        }
        encoder.resolve_query_set(query_set, 0..self.marks, resolve, 0);
        encoder.copy_buffer_to_buffer(resolve, 0, readback, 0, u64::from(self.marks) * TICK_BYTES);
        self.in_flight = self.labels.clone();
        self.state = Readback::Copied;
    }

    /// Call right after the frame's command buffers were submitted.
    pub fn after_submit(&mut self) {
        if !matches!(self.state, Readback::Copied) {
            return;
        }
        let Some(readback) = &self.readback else {
            return;
        };
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let bytes = (self.in_flight.len() as u64 + 1) * TICK_BYTES;
        readback
            .slice(0..bytes)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    flag.store(true, Ordering::Release);
                }
            });
        self.state = Readback::Mapping(done);
    }

    fn harvest(&mut self) {
        let Some(readback) = &self.readback else {
            return;
        };
        let bytes = (self.in_flight.len() as u64 + 1) * TICK_BYTES;
        let ticks: Vec<u64> = {
            let view = readback.slice(0..bytes).get_mapped_range();
            view.chunks_exact(TICK_BYTES as usize)
                .map(|c| u64::from_le_bytes(c.try_into().expect("8-byte chunk")))
                .collect()
        };
        readback.unmap();
        self.timings = durations(&self.in_flight, &ticks, self.period_ns);
        self.state = Readback::Idle;
    }
}

/// Converts raw timestamps into per-label durations (`ticks.len() == labels.len() + 1`).
pub fn durations(labels: &[&'static str], ticks: &[u64], period_ns: f32) -> Vec<GpuTiming> {
    labels
        .iter()
        .zip(ticks.windows(2))
        .map(|(label, pair)| GpuTiming {
            label,
            ms: pair[1].saturating_sub(pair[0]) as f32 * period_ns / 1.0e6,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_attribute_each_interval_to_its_label() {
        let t = durations(&["shadows", "forward"], &[1_000, 3_000, 4_000], 1_000.0);
        assert_eq!(t.len(), 2);
        assert_eq!(
            t[0],
            GpuTiming {
                label: "shadows",
                ms: 2.0
            }
        );
        assert_eq!(
            t[1],
            GpuTiming {
                label: "forward",
                ms: 1.0
            }
        );
    }

    #[test]
    fn non_monotonic_ticks_clamp_to_zero() {
        let t = durations(&["a"], &[5, 3], 1.0e6);
        assert_eq!(t[0].ms, 0.0);
    }

    #[test]
    fn missing_ticks_yield_fewer_timings() {
        assert!(durations(&["a", "b"], &[1], 1.0).is_empty());
    }
}

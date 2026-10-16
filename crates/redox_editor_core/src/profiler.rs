//! Rolling frame-time profiler for the editor status / Profiler tab.

use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct FrameProfiler {
    pub samples_ms: VecDeque<f32>,
    pub capacity: usize,
}

impl Default for FrameProfiler {
    fn default() -> Self {
        Self::new(240)
    }
}

impl FrameProfiler {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples_ms: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push_ms(&mut self, ms: f32) {
        if self.samples_ms.len() >= self.capacity {
            self.samples_ms.pop_front();
        }
        self.samples_ms.push_back(ms.max(0.01));
    }

    pub fn avg_ms(&self) -> f32 {
        if self.samples_ms.is_empty() {
            return 0.0;
        }
        self.samples_ms.iter().sum::<f32>() / self.samples_ms.len() as f32
    }

    pub fn fps(&self) -> f32 {
        let ms = self.avg_ms();
        if ms < 1e-4 {
            0.0
        } else {
            1000.0 / ms
        }
    }

    /// Approximate 1% low FPS (slowest 1% of frames → FPS).
    pub fn one_percent_low_fps(&self) -> f32 {
        if self.samples_ms.len() < 10 {
            return self.fps();
        }
        let mut v: Vec<f32> = self.samples_ms.iter().copied().collect();
        v.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let n = (v.len() as f32 * 0.01).ceil() as usize;
        let n = n.max(1);
        let worst: f32 = v[..n].iter().sum::<f32>() / n as f32;
        1000.0 / worst.max(0.01)
    }

    pub fn point_low_fps(&self) -> f32 {
        if self.samples_ms.len() < 20 {
            return self.one_percent_low_fps();
        }
        let mut v: Vec<f32> = self.samples_ms.iter().copied().collect();
        v.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let n = (v.len() as f32 * 0.001).ceil() as usize;
        let n = n.max(1);
        let worst: f32 = v[..n].iter().sum::<f32>() / n as f32;
        1000.0 / worst.max(0.01)
    }
}

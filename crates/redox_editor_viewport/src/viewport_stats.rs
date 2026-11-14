//! CPU-side stats strip for the viewport overlay (GPU time only when measured).

/// One line of viewport statistics.
#[derive(Clone, Debug, Default)]
pub struct ViewportStatsOverlay {
    pub fps: f32,
    pub cpu_frame_ms: f32,
    /// `None` when `gpu_profiler` did not produce timings this frame.
    pub gpu_frame_ms: Option<f32>,
    pub shading_line: String,
    pub missing_mesh_count: usize,
    pub toggle_hints: Vec<String>,
}

impl ViewportStatsOverlay {
    /// Formatted lines for the integrator to paint in egui or a debug layer.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("FPS: {:.0}", self.fps),
            format!("CPU: {:.2} ms", self.cpu_frame_ms),
        ];
        match self.gpu_frame_ms {
            Some(ms) => out.push(format!("GPU: {ms:.2} ms")),
            None => out.push("GPU: n/a".into()),
        }
        if !self.shading_line.is_empty() {
            out.push(self.shading_line.clone());
        }
        if self.missing_mesh_count > 0 {
            out.push(format!(
                "Missing mesh: {} entit(ies) on proxy fallback",
                self.missing_mesh_count
            ));
        }
        out.extend(self.toggle_hints.iter().cloned());
        out
    }
}
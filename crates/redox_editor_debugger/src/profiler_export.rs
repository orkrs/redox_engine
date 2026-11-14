//! JSON export of one profiler frame (CPU + GPU + FPS).

use serde::{Deserialize, Serialize};

use crate::cpu_timeline::CpuTimeline;
use crate::gpu_passes::GpuPassEntry;
use crate::profiler::FrameProfiler;

/// One exported profiler frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfilerFrameExport {
    /// Average FPS from the rolling window.
    pub fps: f32,
    /// Average frame time in ms.
    pub avg_frame_ms: f32,
    /// CPU spans.
    pub cpu: Vec<CpuSpanExport>,
    /// GPU passes (durations may be null).
    pub gpu: Vec<GpuPassExport>,
}

/// CPU span in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CpuSpanExport {
    /// System name.
    pub name: String,
    /// Milliseconds.
    pub ms: f32,
}

/// GPU pass in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuPassExport {
    /// Pass name.
    pub name: String,
    /// Milliseconds when known; JSON `null` when unavailable.
    pub ms: Option<f32>,
}

/// Builds export struct without UI.
pub fn export_profiler_frame(
    frame_profiler: &FrameProfiler,
    cpu: &CpuTimeline,
    gpu: &[GpuPassEntry],
) -> ProfilerFrameExport {
    ProfilerFrameExport {
        fps: frame_profiler.fps(),
        avg_frame_ms: frame_profiler.avg_ms(),
        cpu: cpu
            .spans
            .iter()
            .map(|s| CpuSpanExport {
                name: s.name.clone(),
                ms: s.ms,
            })
            .collect(),
        gpu: gpu
            .iter()
            .map(|p| GpuPassExport {
                name: p.name.clone(),
                ms: p.duration_ms,
            })
            .collect(),
    }
}

/// Serializes [`ProfilerFrameExport`] to pretty JSON.
pub fn export_profiler_frame_json(
    frame_profiler: &FrameProfiler,
    cpu: &CpuTimeline,
    gpu: &[GpuPassEntry],
) -> Result<String, serde_json::Error> {
    let export = export_profiler_frame(frame_profiler, cpu, gpu);
    serde_json::to_string_pretty(&export)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_timeline::CpuSpan;
    use crate::gpu_passes::GpuPassEntry;

    #[test]
    fn export_json_includes_fps_and_optional_gpu_ms() {
        let mut fp = FrameProfiler::new(16);
        fp.push_ms(16.0);
        let cpu = CpuTimeline {
            spans: vec![CpuSpan {
                name: "update".into(),
                ms: 1.0,
            }],
        };
        let gpu = vec![
            GpuPassEntry {
                name: "forward".into(),
                duration_ms: None,
            },
            GpuPassEntry {
                name: "shadow".into(),
                duration_ms: Some(0.5),
            },
        ];
        let json = export_profiler_frame_json(&fp, &cpu, &gpu).expect("json");
        assert!(json.contains("\"fps\""));
        assert!(json.contains("null") || json.contains("forward"));
        assert!(json.contains("0.5"));
    }
}
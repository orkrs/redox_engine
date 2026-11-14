//! Benchmark runner JSON schema (headless coordinator executes frames).

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::cpu_timeline::CpuTimeline;
use crate::profiler::FrameProfiler;

/// Metadata written with every benchmark report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchMetadata {
    /// Best-effort GPU name (or "n/a").
    pub gpu_name: String,
    /// Driver string (or "n/a").
    pub driver: String,
    /// Git commit hash when available.
    pub git_commit: Option<String>,
    /// ISO-8601-ish timestamp.
    pub timestamp: String,
    /// Frames captured.
    pub frames: u32,
    /// Project path benchmarked.
    pub project: String,
}

/// One frame sample in a benchmark run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchFrameSample {
    /// Frame index.
    pub index: u32,
    /// Frame time ms.
    pub frame_ms: f32,
    /// CPU total ms for the frame (sum of spans).
    pub cpu_total_ms: f32,
}

/// Full benchmark output file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    /// Run metadata.
    pub meta: BenchMetadata,
    /// Per-frame samples (dummy timings OK in unit tests).
    pub samples: Vec<BenchFrameSample>,
    /// Summary FPS.
    pub avg_fps: f32,
}

impl BenchReport {
    /// Builds a report from dummy profiler data (no real GPU).
    pub fn from_dummy_frames(project: &str, frames: u32) -> Self {
        let mut profiler = FrameProfiler::new(frames as usize);
        let mut samples = Vec::with_capacity(frames as usize);
        for i in 0..frames {
            let ms = 16.0 + (i as f32 * 0.01);
            profiler.push_ms(ms);
            let cpu = CpuTimeline {
                spans: vec![],
            };
            samples.push(BenchFrameSample {
                index: i,
                frame_ms: ms,
                cpu_total_ms: cpu.total_ms(),
            });
        }
        Self {
            meta: BenchMetadata {
                gpu_name: "n/a".into(),
                driver: "n/a".into(),
                git_commit: None,
                timestamp: "1970-01-01T00:00:00Z".into(),
                frames,
                project: project.to_string(),
            },
            samples,
            avg_fps: profiler.fps(),
        }
    }

    /// Writes JSON to `path`.
    pub fn write_json(&self, path: &Path) -> Result<(), std::io::Error> {
        let text =
            serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, text)
    }

    /// Optional minimal HTML wrapper around the JSON file name.
    pub fn write_html_stub(&self, path: &Path) -> Result<(), std::io::Error> {
        let body = format!(
            "<!doctype html><html><head><title>RedOx Bench</title></head>\
<body><h1>Benchmark {} frames</h1><p>Avg FPS {:.1}</p>\
<p>Load <code>{}.json</code> for full data.</p></body></html>",
            self.meta.frames,
            self.avg_fps,
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("report")
        );
        std::fs::write(path, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn bench_json_contains_schema_fields_on_dummy_timings() {
        let report = BenchReport::from_dummy_frames("/tmp/proj", 3);
        let json = serde_json::to_string(&report).expect("serde");
        assert!(json.contains("gpu_name"));
        assert!(json.contains("git_commit"));
        assert!(json.contains("samples"));
        assert!(json.contains("n/a"));
        let dir = std::env::temp_dir().join(format!("redox_bench_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("report.json");
        report.write_json(&path).expect("write");
        let read: BenchReport =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("parse");
        assert_eq!(read.meta.frames, 3);
        let _ = fs::remove_dir_all(&dir);
    }
}
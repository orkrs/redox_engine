//! GPU pass list model from frame graph report + optional wgpu timings.

use redox_render::profiler::GpuTiming;
use serde::{Deserialize, Serialize};

/// One render pass row in the profiler panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuPassEntry {
    /// Pass name from the frame graph / GPU profiler.
    pub name: String,
    /// GPU duration when timestamps are available; otherwise `None` (display "n/a").
    pub duration_ms: Option<f32>,
}

/// Parses `frame_graph_report` / compiled graph report text into pass names.
pub fn passes_from_frame_graph_report(report: &str) -> Vec<GpuPassEntry> {
    let mut out = Vec::new();
    for line in report.lines() {
        let trimmed = line.trim();
        let name = if let Some(rest) = trimmed.strip_prefix("pass ") {
            rest.split_whitespace().next().unwrap_or(rest)
        } else if let Some((_idx, rest)) = trimmed.split_once(". ") {
            rest.split_whitespace().next().unwrap_or(rest)
        } else {
            continue;
        };
        let name = name.to_string();
        if !out.iter().any(|e| e.name == name) {
            out.push(GpuPassEntry {
                name,
                duration_ms: None,
            });
        }
    }
    if out.is_empty() {
        for name in redox_render::frame_pass_order() {
            out.push(GpuPassEntry {
                name: (*name).to_string(),
                duration_ms: None,
            });
        }
    }
    out
}

/// Merges wgpu timestamp timings into the pass list by label (no invented durations).
pub fn merge_gpu_timings(entries: &mut [GpuPassEntry], timings: &[GpuTiming]) {
    for entry in entries.iter_mut() {
        let ms = timings
            .iter()
            .find(|t| t.label == entry.name)
            .map(|t| t.ms);
        entry.duration_ms = ms;
    }
}

/// Human-readable duration or `n/a`.
pub fn format_duration_ms(entry: &GpuPassEntry) -> &'static str {
    if entry.duration_ms.is_some() {
        "ms"
    } else {
        "n/a"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r"
pass shadow_passes
  read shadow_maps
pass cluster_vg_cull
pass frame_tail
";

    #[test]
    fn parse_report_fixture_lists_passes_without_durations() {
        let passes = passes_from_frame_graph_report(FIXTURE);
        assert!(passes.iter().all(|p| p.duration_ms.is_none()));
        assert!(passes.iter().any(|p| p.name == "shadow_passes"));
        assert!(passes.iter().any(|p| p.name == "frame_tail"));
    }

    #[test]
    fn merge_timings_only_when_present() {
        let mut passes = passes_from_frame_graph_report(FIXTURE);
        let timings = [GpuTiming {
            label: "shadow_passes",
            ms: 1.5,
        }];
        merge_gpu_timings(&mut passes, &timings);
        let shadow = passes.iter().find(|p| p.name == "shadow_passes").unwrap();
        assert_eq!(shadow.duration_ms, Some(1.5));
        let tail = passes.iter().find(|p| p.name == "frame_tail").unwrap();
        assert_eq!(tail.duration_ms, None);
    }

    #[test]
    fn unavailable_backend_keeps_none() {
        let passes = passes_from_frame_graph_report("no passes here");
        assert!(!passes.is_empty());
        assert!(passes.iter().all(|p| p.duration_ms.is_none()));
    }
}
//! Play-in-editor, time travel and frame profiling of RedOx Studio.

#![warn(missing_docs)]

/// Play-in-editor session control.
pub mod pie;
/// PIE start scene and argument settings.
pub mod pie_settings;
/// Apply PIE settings to the sandbox world before Play.
pub mod pie_prepare;
/// Standalone play launch plan (no subprocess from the library).
pub mod standalone;
/// Console filtering and export model.
pub mod console;
/// Runtime entity search during PIE.
pub mod runtime_inspector;
/// Rolling frame-time profiler.
pub mod profiler;
/// CPU timeline from dispatcher spans.
pub mod cpu_timeline;
/// GPU pass list model.
pub mod gpu_passes;
/// Profiler JSON export.
pub mod profiler_export;
/// Benchmark runner schema.
pub mod benchmark;
/// Ring buffer of world snapshots for scrubbing back in time.
pub mod time_travel;

pub use pie::{PieRunMode, PieSession};
pub use pie_settings::{PieArgs, PieSettings};
pub use pie_prepare::{PieSceneLoad, prepare_pie_world};
pub use standalone::{StandalonePlan, plan_standalone};
pub use console::{ConsoleEntry, ConsoleFilter, ConsoleModel, LogLevel};
pub use runtime_inspector::{
    RuntimeInspectorHighlight, RuntimeSearchHit, RuntimeSearchQuery, search_runtime_entities,
};
pub use profiler::FrameProfiler;
pub use cpu_timeline::{CpuSpan, CpuTimeline};
pub use gpu_passes::{GpuPassEntry, merge_gpu_timings, passes_from_frame_graph_report};
pub use profiler_export::{
    ProfilerFrameExport, export_profiler_frame, export_profiler_frame_json,
};
pub use benchmark::{BenchMetadata, BenchReport};
pub use time_travel::TimeTravelBuffer;

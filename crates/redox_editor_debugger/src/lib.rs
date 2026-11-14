//! Play-in-editor, time travel and frame profiling of RedOx Studio.

#![warn(missing_docs)]

/// Benchmark runner schema.
pub mod benchmark;
/// Console filtering and export model.
pub mod console;
/// CPU timeline from dispatcher spans.
pub mod cpu_timeline;
/// GPU pass list model.
pub mod gpu_passes;
/// Play-in-editor session control.
pub mod pie;
/// Apply PIE settings to the sandbox world before Play.
pub mod pie_prepare;
/// Blocks persistent saves while play-in-editor is running.
pub mod pie_save;
/// PIE start scene and argument settings.
pub mod pie_settings;
/// Rolling frame-time profiler.
pub mod profiler;
/// Profiler JSON export.
pub mod profiler_export;
/// Runtime entity search during PIE.
pub mod runtime_inspector;
/// Standalone play launch plan (no subprocess from the library).
pub mod standalone;
/// Ring buffer of world snapshots for scrubbing back in time.
pub mod time_travel;

pub use benchmark::{BenchMetadata, BenchReport};
pub use console::{ConsoleEntry, ConsoleFilter, ConsoleModel, LogLevel};
pub use cpu_timeline::{CpuSpan, CpuTimeline};
pub use gpu_passes::{GpuPassEntry, merge_gpu_timings, passes_from_frame_graph_report};
pub use pie::{PieRunMode, PieSession};
pub use pie_prepare::{PieSceneLoad, prepare_pie_world};
pub use pie_save::{PieSaveBlock, mark_saved_if_allowed, save_guard, save_level_if_allowed};
pub use pie_settings::{PieArgs, PieSettings};
pub use profiler::FrameProfiler;
pub use profiler_export::{ProfilerFrameExport, export_profiler_frame, export_profiler_frame_json};
pub use runtime_inspector::{
    RuntimeInspectorHighlight, RuntimeSearchHit, RuntimeSearchQuery, search_runtime_entities,
};
pub use standalone::{StandalonePlan, plan_standalone};
pub use time_travel::TimeTravelBuffer;

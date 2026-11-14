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
/// Rolling frame-time profiler.
pub mod profiler;
/// Ring buffer of world snapshots for scrubbing back in time.
pub mod time_travel;

pub use pie::{PieRunMode, PieSession};
pub use pie_settings::{PieArgs, PieSettings};
pub use pie_prepare::{PieSceneLoad, prepare_pie_world};
pub use standalone::{StandalonePlan, plan_standalone};
pub use profiler::FrameProfiler;
pub use time_travel::TimeTravelBuffer;

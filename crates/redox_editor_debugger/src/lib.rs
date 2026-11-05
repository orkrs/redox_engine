//! Play-in-editor, time travel and frame profiling of RedOx Studio.

#![warn(missing_docs)]

/// Play-in-editor session control.
pub mod pie;
/// Rolling frame-time profiler.
pub mod profiler;
/// Ring buffer of world snapshots for scrubbing back in time.
pub mod time_travel;

pub use pie::PieSession;
pub use profiler::FrameProfiler;
pub use time_travel::TimeTravelBuffer;

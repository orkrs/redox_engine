//! UI subsystem for the RedOx Engine.
//!
//! Provides:
//! - [`UiContext`] — egui + wgpu + winit integration layer
//! - [`DebugOverlay`] — master debug overlay (owns stats + inspector)
//! - [`StatsPanel`] — performance statistics with FPS graph
//! - [`EntityInspector`] — ECS entity / archetype browser

pub mod context;
pub mod debug;

pub use context::UiContext;
pub use debug::DebugOverlay;
pub use debug::stats::StatsPanel;
pub use debug::inspector::EntityInspector;

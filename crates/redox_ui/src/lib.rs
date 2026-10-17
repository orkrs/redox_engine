//! UI subsystem for the RedOx Engine.
//!
//! Provides:
//! - [`UiContext`] — egui + wgpu + winit integration layer
//! - [`DebugOverlay`] — master debug overlay (owns stats + inspector)
//! - [`StatsPanel`] — performance statistics with FPS graph
//! - [`EntityInspector`] — ECS entity / archetype browser

#![warn(missing_docs)]

pub mod context;
pub mod debug;
pub mod game;

pub use context::UiContext;
pub use debug::DebugOverlay;
pub use debug::inspector::EntityInspector;
pub use debug::stats::StatsPanel;

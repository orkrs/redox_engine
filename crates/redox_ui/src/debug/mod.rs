//! Debug overlay — groups [`StatsPanel`] and [`EntityInspector`] together.
//!
//! [`DebugOverlay`] is the top-level struct that lives inside [`UiContext`].
//! It provides:
//! - A persistent control bar ("RedOx Debug") with toggle buttons and F-key hints.
//! - The [`StatsPanel`] (performance statistics + FPS graph).
//! - The [`EntityInspector`] (ECS entity / archetype browser).
//!
//! # Usage
//! ```rust,ignore
//! // Once per frame, pass dt before building any UI widgets:
//! ui_ctx.debug.record_frame(dt);
//!
//! // Inside the egui frame:
//! ui_ctx.debug.show(ctx, &world);
//! ```

pub mod stats;
pub mod inspector;

pub use stats::StatsPanel;
pub use inspector::EntityInspector;

use redox_ecs::world::World;

// ─── DebugOverlay ─────────────────────────────────────────────────────────────

/// Top-level debug overlay that owns all debug panels.
///
/// Embed this inside [`UiContext`](crate::context::UiContext) and drive it each
/// frame with [`record_frame`](Self::record_frame) + [`show`](Self::show).
pub struct DebugOverlay {
    /// Performance statistics window (FPS graph, low percentiles, …).
    pub stats: StatsPanel,
    /// ECS entity / archetype inspector window.
    pub inspector: EntityInspector,
    /// Master switch — when `false` no debug windows are drawn at all.
    pub enabled: bool,
}

impl DebugOverlay {
    /// Creates a new overlay with both panels open.
    pub fn new() -> Self {
        Self {
            stats: StatsPanel::new(),
            inspector: EntityInspector::new(),
            enabled: true,
        }
    }

    /// Records one frame's delta time (seconds).
    ///
    /// Must be called once per frame, **before** [`show`](Self::show).
    #[inline]
    pub fn record_frame(&mut self, dt: f32) {
        self.stats.record_frame(dt);
    }

    /// Draws all open debug windows and the control bar.
    ///
    /// Call once per egui frame (between `begin_frame` and `end_frame_and_render`).
    ///
    /// `world` is passed through to the inspector; the stats panel only needs
    /// timing data that has already been recorded via [`record_frame`].
    pub fn show(&mut self, ctx: &egui::Context, world: &World) {
        if !self.enabled {
            return;
        }

        self.draw_control_bar(ctx);

        if self.stats.open {
            self.stats.show(ctx);
        }
        if self.inspector.open {
            self.inspector.show(ctx, world);
        }
    }

    // ── control bar ─────────────────────────────────────────────────────────

    /// Draws a small toolbar that lets the user toggle panels.
    ///
    /// The bar itself cannot be closed — it is the entry point to reach the
    /// other windows.  Position it in the bottom-left corner so it stays out
    /// of the way of the main game HUD.
    fn draw_control_bar(&mut self, ctx: &egui::Context) {
        egui::Window::new("🛠 Debug")
            .anchor(egui::Align2::LEFT_BOTTOM, [8.0, -8.0])
            .resizable(false)
            .collapsible(true)
            .default_open(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let stats_label = if self.stats.open { "📊 Stats ✓" } else { "📊 Stats" };
                    if ui.small_button(stats_label).clicked() {
                        self.stats.open = !self.stats.open;
                    }

                    let insp_label = if self.inspector.open {
                        "🔍 Inspector ✓"
                    } else {
                        "🔍 Inspector"
                    };
                    if ui.small_button(insp_label).clicked() {
                        self.inspector.open = !self.inspector.open;
                    }
                });

                ui.weak("F1 Stats · F2 Inspector · F3 Debug");
            });
    }
}

impl Default for DebugOverlay {
    fn default() -> Self {
        Self::new()
    }
}

//! Performance statistics panel for the debug overlay.
//!
//! [`StatsPanel`] accumulates per-frame timing data in a ring buffer and
//! renders it as an egui window that includes:
//!
//! - Current / average FPS
//! - Frame time in milliseconds
//! - 0.1 % and 1 % low FPS (worst-case percentiles, like CapFrameX)
//! - Live line-chart of FPS over the last few seconds (via `egui_plot`)
//!
//! # Example
//! ```rust,ignore
//! // Once per frame, before drawing UI:
//! stats_panel.record_frame(delta_time_secs);
//!
//! // Inside egui frame:
//! stats_panel.show(ctx);
//! ```

use std::collections::VecDeque;
use egui_plot::{Line, Plot, PlotPoints};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Number of frame-time samples kept in the ring buffer (≈ 5 s at 60 fps).
const MAX_SAMPLES: usize = 300;

/// Samples used for the "average FPS" label shown at the top.
const AVG_WINDOW: usize = 60;

// ─── StatsPanel ───────────────────────────────────────────────────────────────

/// Persistent state for the performance statistics window.
///
/// Create once, call [`record_frame`](StatsPanel::record_frame) every frame
/// (before UI begins), then call [`show`](StatsPanel::show) inside an egui
/// frame to render the window.
pub struct StatsPanel {
    /// Whether the window is currently open.
    pub open: bool,

    /// Ring buffer of the most recent frame times, in **seconds**.
    frame_times: VecDeque<f32>,

    /// Total elapsed time since the game started, in seconds.
    total_time: f32,
}

impl StatsPanel {
    /// Creates a new panel with the window open by default.
    pub fn new() -> Self {
        Self {
            open: true,
            frame_times: VecDeque::with_capacity(MAX_SAMPLES),
            total_time: 0.0,
        }
    }

    /// Records one frame's delta time (in **seconds**).
    ///
    /// Must be called exactly once per frame, before calling [`show`](Self::show).
    pub fn record_frame(&mut self, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        self.total_time += dt;
        if self.frame_times.len() == MAX_SAMPLES {
            self.frame_times.pop_front();
        }
        self.frame_times.push_back(dt);
    }

    // ── derived statistics ──────────────────────────────────────────────────

    /// Instantaneous FPS (reciprocal of the last frame time).
    fn current_fps(&self) -> f32 {
        self.frame_times
            .back()
            .map(|&dt| 1.0 / dt)
            .unwrap_or(0.0)
    }

    /// Average FPS over the last [`AVG_WINDOW`] frames.
    fn avg_fps(&self) -> f32 {
        let window: Vec<f32> = self
            .frame_times
            .iter()
            .rev()
            .take(AVG_WINDOW)
            .copied()
            .collect();
        if window.is_empty() {
            return 0.0;
        }
        let avg_dt = window.iter().sum::<f32>() / window.len() as f32;
        1.0 / avg_dt
    }

    /// 1 % low FPS — average of the worst 1 % of frame times.
    ///
    /// A high frame time → low FPS; a low 1 % low means occasional big spikes.
    fn low_fps(&self, percentile: f32) -> f32 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<f32> = self.frame_times.iter().copied().collect();
        // Sort descending by frame time (worst frames first).
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());

        // How many frames constitute the given percentile (at least 1).
        let count = ((sorted.len() as f32 * percentile / 100.0).ceil() as usize).max(1);
        let worst_avg = sorted[..count].iter().sum::<f32>() / count as f32;
        1.0 / worst_avg
    }

    /// The complete FPS history as a `Vec` ordered oldest → newest.
    fn fps_history(&self) -> Vec<f64> {
        self.frame_times
            .iter()
            .map(|&dt| (1.0 / dt) as f64)
            .collect()
    }

    // ── rendering ──────────────────────────────────────────────────────────

    /// Draws the statistics window. Call once per egui frame.
    ///
    /// The window can be closed with the × button; re-open by setting
    /// [`open`](Self::open) to `true` or toggling it from the overlay control bar.
    pub fn show(&mut self, ctx: &egui::Context) {
        let mut open = self.open;
        egui::Window::new("📊 Performance")
            .open(&mut open)
            .resizable(true)
            .default_pos([10.0, 10.0])
            .default_size([340.0, 300.0])
            .show(ctx, |ui| {
                self.draw_contents(ui);
            });
        self.open = open;
    }

    fn draw_contents(&self, ui: &mut egui::Ui) {
        let fps = self.current_fps();
        let avg = self.avg_fps();
        let low_1 = self.low_fps(1.0);
        let low_01 = self.low_fps(0.1);
        let last_dt_ms = self.frame_times.back().copied().unwrap_or(0.0) * 1000.0;

        // ── text summary ────────────────────────────────────────────────────
        egui::Grid::new("stats_grid")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("FPS (current)");
                ui.label(
                    egui::RichText::new(format!("{fps:.0}"))
                        .color(fps_color(fps))
                        .strong(),
                );
                ui.end_row();

                ui.label("FPS (avg 60f)");
                ui.label(format!("{avg:.1}"));
                ui.end_row();

                ui.label("Frame time");
                ui.label(format!("{last_dt_ms:.2} ms"));
                ui.end_row();

                ui.label("1 % low");
                ui.label(
                    egui::RichText::new(format!("{low_1:.1}"))
                        .color(fps_color(low_1)),
                );
                ui.end_row();

                ui.label("0.1 % low");
                ui.label(
                    egui::RichText::new(format!("{low_01:.1}"))
                        .color(fps_color(low_01)),
                );
                ui.end_row();

                ui.label("Uptime");
                ui.label(format!("{:.1} s", self.total_time));
                ui.end_row();
            });

        ui.separator();

        // ── FPS line chart ──────────────────────────────────────────────────
        let history = self.fps_history();
        if history.len() >= 2 {
            let points: PlotPoints = history
                .iter()
                .enumerate()
                .map(|(i, &v)| [i as f64, v])
                .collect();

            let line = Line::new(points)
                .color(egui::Color32::from_rgb(100, 200, 255))
                .name("FPS");

            Plot::new("fps_plot")
                .height(100.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .include_y(0.0)
                .y_axis_label("FPS")
                .show_axes([false, true])
                .show(ui, |plot_ui| {
                    plot_ui.line(line);
                });
        } else {
            ui.label("Collecting samples…");
        }
    }
}

impl Default for StatsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Returns a colour that goes green → yellow → red as FPS drops.
fn fps_color(fps: f32) -> egui::Color32 {
    if fps >= 55.0 {
        egui::Color32::from_rgb(80, 220, 80)
    } else if fps >= 30.0 {
        egui::Color32::from_rgb(230, 180, 40)
    } else {
        egui::Color32::from_rgb(220, 60, 60)
    }
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn filled_panel(n: usize, dt: f32) -> StatsPanel {
        let mut p = StatsPanel::new();
        for _ in 0..n {
            p.record_frame(dt);
        }
        p
    }

    #[test]
    fn current_fps_correct() {
        let p = filled_panel(10, 0.016);
        assert!((p.current_fps() - 62.5).abs() < 1.0);
    }

    #[test]
    fn low_fps_at_most_avg() {
        let p = filled_panel(MAX_SAMPLES, 0.016);
        assert!(p.low_fps(1.0) <= p.avg_fps() + 0.1);
    }

    #[test]
    fn ring_buffer_capped() {
        let mut p = StatsPanel::new();
        for _ in 0..(MAX_SAMPLES + 50) {
            p.record_frame(0.016);
        }
        assert_eq!(p.frame_times.len(), MAX_SAMPLES);
    }

    #[test]
    fn zero_dt_ignored() {
        let mut p = StatsPanel::new();
        p.record_frame(0.0);
        assert!(p.frame_times.is_empty());
    }
}

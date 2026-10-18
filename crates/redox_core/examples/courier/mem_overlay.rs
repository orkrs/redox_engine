//! State of the GPU memory overlay (F5): visibility and a text that is refreshed at most twice a second.
//!
//! SPECIFICATION FILE (tests first). Bodies marked `todo!()` are to be implemented so that the tests at
//! the bottom pass; do not change the tests or the signatures. The report text comes from
//! `Renderer::memory_report()`; [`draw`] paints it on screen and is called from `main.rs` of Courier and
//! Foxfire (the Foxfire example includes this very file with `#[path = "../courier/mem_overlay.rs"] mod mem_overlay;`).
//!
//! Rules:
//! - The overlay starts hidden with an empty text.
//! - `toggle` flips visibility. Hiding clears the text. Showing arms an immediate refresh.
//! - `update(dt, report)`: when hidden, does nothing and never calls `report`. When visible, calls
//!   `report` and stores the result if no text has been fetched yet since showing, or at least
//!   [`REFRESH_SECONDS`] of accumulated `dt` have passed since the last fetch. Negative or NaN `dt`
//!   counts as zero.

#![allow(dead_code)]

/// Seconds between two refreshes of the text.
pub const REFRESH_SECONDS: f32 = 0.5;

/// Visibility and cached text of the memory overlay.
#[derive(Clone, Debug, Default)]
pub struct MemoryOverlay {
    visible: bool,
    since_fetch: f32,
    text: String,
    fetched: bool,
}

impl MemoryOverlay {
    /// A hidden overlay.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the overlay is shown.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Shows or hides the overlay.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.fetched = false;
        self.since_fetch = 0.0;
        if !self.visible {
            self.text.clear();
        }
    }

    /// Advances the refresh timer and fetches a new report when due (see the rules above).
    pub fn update(&mut self, dt: f32, report: impl FnOnce() -> String) {
        if !self.visible {
            return;
        }
        let step = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        self.since_fetch += step;
        if !self.fetched || self.since_fetch >= REFRESH_SECONDS {
            self.text = report();
            self.fetched = true;
            self.since_fetch = 0.0;
        }
    }

    /// The text to draw (empty while hidden or before the first fetch).
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Gap between the overlay box and the window edges, and inside the box, in egui points.
const MARGIN: f32 = 12.0;
const PADDING: f32 = 6.0;
const FONT_SIZE: f32 = 13.0;

/// Draws the overlay text in the bottom-left corner of `screen` on a translucent box.
/// Draws nothing while the overlay is hidden or the text has not been fetched yet.
pub fn draw(overlay: &MemoryOverlay, painter: &egui::Painter, screen: egui::Rect) {
    if !overlay.visible() || overlay.text().is_empty() {
        return;
    }
    let galley = painter.layout_no_wrap(
        overlay.text().to_string(),
        egui::FontId::monospace(FONT_SIZE),
        egui::Color32::from_gray(235),
    );
    let boxed = galley.size() + egui::vec2(PADDING, PADDING) * 2.0;
    let min = egui::pos2(screen.left() + MARGIN, screen.bottom() - MARGIN - boxed.y);
    let rect = egui::Rect::from_min_size(min, boxed);
    painter.rect_filled(rect, 4.0, egui::Color32::from_black_alpha(160));
    painter.galley(
        min + egui::vec2(PADDING, PADDING),
        galley,
        egui::Color32::from_gray(235),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_hidden_and_empty() {
        let overlay = MemoryOverlay::new();
        assert!(!overlay.visible());
        assert_eq!(overlay.text(), "");
    }

    #[test]
    fn a_hidden_overlay_never_asks_for_a_report() {
        let mut overlay = MemoryOverlay::new();
        overlay.update(10.0, || panic!("hidden overlays must not build reports"));
        assert_eq!(overlay.text(), "");
    }

    #[test]
    fn showing_fetches_at_the_next_update_and_hiding_clears() {
        let mut overlay = MemoryOverlay::new();
        overlay.toggle();
        assert!(overlay.visible());
        assert_eq!(overlay.text(), "", "nothing before the first update");
        overlay.update(0.0, || "textures: 1.0 MiB".to_string());
        assert_eq!(overlay.text(), "textures: 1.0 MiB");
        overlay.toggle();
        assert!(!overlay.visible());
        assert_eq!(overlay.text(), "");
    }

    #[test]
    fn the_text_is_refreshed_at_most_every_half_second() {
        let mut overlay = MemoryOverlay::new();
        overlay.toggle();
        let mut calls = 0;
        let mut fetch = |overlay: &mut MemoryOverlay, dt: f32| {
            overlay.update(dt, || {
                calls += 1;
                format!("report {calls}")
            });
        };
        fetch(&mut overlay, 0.1);
        fetch(&mut overlay, 0.2);
        fetch(&mut overlay, 0.1);
        assert_eq!(overlay.text(), "report 1", "0.4 s since the first fetch");
        fetch(&mut overlay, 0.2);
        assert_eq!(overlay.text(), "report 2", "0.6 s have passed");
        fetch(&mut overlay, 0.1);
        assert_eq!(overlay.text(), "report 2");
    }

    #[test]
    fn bad_time_steps_count_as_zero() {
        let mut overlay = MemoryOverlay::new();
        overlay.toggle();
        overlay.update(0.0, || "a".to_string());
        overlay.update(-5.0, || panic!("negative time must not trigger a refresh"));
        overlay.update(f32::NAN, || panic!("NaN time must not trigger a refresh"));
        assert_eq!(overlay.text(), "a");
    }

    #[test]
    fn showing_again_fetches_immediately() {
        let mut overlay = MemoryOverlay::new();
        overlay.toggle();
        overlay.update(0.0, || "first".to_string());
        overlay.toggle();
        overlay.toggle();
        overlay.update(0.0, || "second".to_string());
        assert_eq!(overlay.text(), "second");
    }
}

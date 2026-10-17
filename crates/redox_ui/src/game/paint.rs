//! Draws a laid-out tree with egui's painter. Fonts, glyph shaping and anti-aliasing come from
//! egui; the layout itself does not depend on it.

use super::layout::{Color, Laid, LaidUi, Rect, UiScale};

fn color32(c: Color) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
}

/// A text measure that uses egui's real fonts, in virtual pixels (for [`super::layout`]).
pub fn egui_measure<'a>(
    painter: &'a egui::Painter,
    scale: UiScale,
) -> impl Fn(&str, f32) -> (f32, f32) + 'a {
    move |text, size| {
        let galley = painter.layout_no_wrap(
            text.to_string(),
            egui::FontId::proportional(size * scale.scale),
            egui::Color32::WHITE,
        );
        let s = galley.size();
        (s.x / scale.scale, s.y / scale.scale)
    }
}

/// Converts a virtual rectangle to egui screen coordinates.
pub fn to_egui_rect(rect: Rect, scale: UiScale) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(rect.x * scale.scale, rect.y * scale.scale),
        egui::vec2(rect.w * scale.scale, rect.h * scale.scale),
    )
}

/// Paints every item of `ui`. The button whose id equals `focused` gets a frame in its focus colour.
pub fn paint(ui: &LaidUi, painter: &egui::Painter, scale: UiScale, focused: Option<&str>) {
    for item in &ui.items {
        let rect = to_egui_rect(item.rect, scale);
        let style = &item.style;
        let radius = egui::CornerRadius::same((6.0 * scale.scale).round().clamp(0.0, 255.0) as u8);
        match &item.kind {
            Laid::Panel => {
                if let Some(bg) = style.background {
                    painter.rect_filled(rect, radius, color32(bg));
                }
            }
            Laid::Label(text) => {
                painter.text(
                    rect.left_top(),
                    egui::Align2::LEFT_TOP,
                    text,
                    egui::FontId::proportional(style.font_size * scale.scale),
                    color32(style.color),
                );
            }
            Laid::Button { id, label } => {
                let is_focused = focused == Some(id.as_str());
                let fill = style.background.map_or(
                    egui::Color32::from_rgba_unmultiplied(
                        255,
                        255,
                        255,
                        if is_focused { 48 } else { 22 },
                    ),
                    color32,
                );
                painter.rect_filled(rect, radius, fill);
                if is_focused {
                    painter.rect_stroke(
                        rect,
                        radius,
                        egui::Stroke::new(3.0 * scale.scale, color32(style.focus_color)),
                        egui::StrokeKind::Inside,
                    );
                }
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(style.font_size * scale.scale),
                    color32(style.color),
                );
            }
            Laid::Bar(value) => {
                painter.rect_filled(rect, radius, egui::Color32::from_black_alpha(140));
                let mut filled = rect;
                filled.set_width(rect.width() * value.clamp(0.0, 1.0));
                painter.rect_filled(filled, radius, color32(style.color));
            }
        }
    }
}

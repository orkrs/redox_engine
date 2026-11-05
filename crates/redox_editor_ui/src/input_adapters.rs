//! Thin egui adapters over the egui-free logic of the editor crates.

use redox_editor_core::UndoHotkey;

/// Maps an egui point inside `rect` to a pixel of the viewport texture (see
/// [`redox_editor_viewport::viewport::window_to_viewport_coords`]); `None` outside the rect.
pub fn egui_to_viewport_coords(
    pos: egui::Pos2,
    rect: egui::Rect,
    _pixels_per_point: f32,
    tex_size: (u32, u32),
) -> Option<(u32, u32)> {
    redox_editor_viewport::window_to_viewport_coords(
        [pos.x, pos.y],
        [rect.min.x, rect.min.y],
        [rect.width(), rect.height()],
        tex_size,
    )
}

/// Recognises Ctrl+Z (undo) and Ctrl+Y or Ctrl+Shift+Z (redo) in the egui input of this frame.
pub fn undo_hotkey(ctx: &egui::Context) -> Option<UndoHotkey> {
    let ctrl_z =
        ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z) && !i.modifiers.shift);
    let ctrl_y = ctx.input(|i| {
        i.modifiers.ctrl
            && (i.key_pressed(egui::Key::Y) || (i.key_pressed(egui::Key::Z) && i.modifiers.shift))
    });
    if ctrl_z {
        Some(UndoHotkey::Undo)
    } else if ctrl_y {
        Some(UndoHotkey::Redo)
    } else {
        None
    }
}

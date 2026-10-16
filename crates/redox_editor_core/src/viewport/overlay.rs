//! CPU-side overlay geometry and coordinate helpers for the viewport.

use redox_math::Vec3;

/// Build grid + world axes + translate gizmo at `gizmo_origin`.
pub fn build_editor_overlay_lines(
    gizmo_origin: Option<Vec3>,
    cell_size: f32,
) -> Vec<(Vec3, Vec3, [f32; 4])> {
    let mut lines = Vec::new();
    let half = 3.0 * cell_size;
    let grid_col = [0.25, 0.28, 0.32, 0.85];
    let axis_x = [0.85, 0.25, 0.25, 1.0];
    let axis_z = [0.25, 0.45, 0.95, 1.0];
    for i in -3..=3 {
        let p = i as f32 * cell_size;
        let col = if i == 0 { axis_x } else { grid_col };
        lines.push((Vec3::new(-half, 0.0, p), Vec3::new(half, 0.0, p), col));
        let col = if i == 0 { axis_z } else { grid_col };
        lines.push((Vec3::new(p, 0.0, -half), Vec3::new(p, 0.0, half), col));
    }
    lines.push((Vec3::ZERO, Vec3::Y * half * 0.35, [0.25, 0.9, 0.35, 1.0]));

    if let Some(o) = gizmo_origin {
        let len = 1.35;
        let tip = 0.18;
        lines.push((o, o + Vec3::X * len, [1.0, 0.2, 0.2, 1.0]));
        lines.push((
            o + Vec3::X * len,
            o + Vec3::X * (len - tip) + Vec3::Y * tip * 0.5,
            [1.0, 0.2, 0.2, 1.0],
        ));
        lines.push((o, o + Vec3::Y * len, [0.25, 1.0, 0.35, 1.0]));
        lines.push((
            o + Vec3::Y * len,
            o + Vec3::Y * (len - tip) + Vec3::X * tip * 0.5,
            [0.25, 1.0, 0.35, 1.0],
        ));
        lines.push((o, o + Vec3::Z * len, [0.3, 0.55, 1.0, 1.0]));
        lines.push((
            o + Vec3::Z * len,
            o + Vec3::Z * (len - tip) + Vec3::Y * tip * 0.5,
            [0.3, 0.55, 1.0, 1.0],
        ));
    }
    lines
}

/// Wireframe unit cube (ghost / git old position).
pub fn ghost_cube_lines(center: Vec3, color: [f32; 4]) -> Vec<(Vec3, Vec3, [f32; 4])> {
    let h = 0.5;
    let c = [
        center + Vec3::new(-h, -h, -h),
        center + Vec3::new(h, -h, -h),
        center + Vec3::new(h, -h, h),
        center + Vec3::new(-h, -h, h),
        center + Vec3::new(-h, h, -h),
        center + Vec3::new(h, h, -h),
        center + Vec3::new(h, h, h),
        center + Vec3::new(-h, h, h),
    ];
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    edges
        .into_iter()
        .map(|(a, b)| (c[a], c[b], color))
        .collect()
}

/// Vector from old → new for git diff.
pub fn ghost_move_line(old: Vec3, new: Vec3) -> (Vec3, Vec3, [f32; 4]) {
    (old, new, [1.0, 0.85, 0.2, 0.9])
}

pub fn egui_to_viewport_coords(
    egui_pos: egui::Pos2,
    viewport_rect: egui::Rect,
    _pixels_per_point: f32,
    tex_size: (u32, u32),
) -> Option<(u32, u32)> {
    // egui positions + rects are in *points*; the texture is stretched to fill the rect.
    // Do NOT multiply by pixels_per_point — that breaks picking on Windows DPI scaling.
    if !viewport_rect.contains(egui_pos) {
        return None;
    }
    let rel = egui_pos - viewport_rect.min;
    let w = viewport_rect.width().max(1e-3);
    let h = viewport_rect.height().max(1e-3);
    let uv_x = (rel.x / w).clamp(0.0, 1.0);
    let uv_y = (rel.y / h).clamp(0.0, 1.0);
    let x = (uv_x * tex_size.0 as f32).floor() as u32;
    let y = (uv_y * tex_size.1 as f32).floor() as u32;
    if x >= tex_size.0 || y >= tex_size.1 {
        None
    } else {
        Some((x.min(tex_size.0 - 1), y.min(tex_size.1 - 1)))
    }
}

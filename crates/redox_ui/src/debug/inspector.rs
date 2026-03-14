//! Entity inspector panel for the debug overlay.
//!
//! [`EntityInspector`] renders an egui window that shows:
//! - All active archetypes (component-type signatures)
//! - All entities grouped under their archetype
//! - Per-entity component values for known types
//!   ([`Transform`], [`MeshHandle`], [`MaterialHandle`])
//!
//! The inspector updates every frame automatically because it reads directly
//! from the [`World`] reference passed to [`EntityInspector::show`].
//!
//! # Extending the inspector
//!
//! To display custom component types, call `world.get_component::<MyType>(entity)`
//! inside the `draw_entity_components` function and add the matching arm.

use std::any::TypeId;
use redox_ecs::world::World;
use redox_math::Transform;
use redox_render::systems::{MeshHandle, MaterialHandle};

// ─── EntityInspector ──────────────────────────────────────────────────────────

/// Persistent state for the entity-inspector debug window.
///
/// Create once at startup, then call [`show`](Self::show) each frame inside
/// an egui frame to render the window.
pub struct EntityInspector {
    /// Whether the window is currently open.
    pub open: bool,

    /// Index of the archetype currently expanded in the UI (`None` = all collapsed).
    selected_archetype: Option<usize>,
}

impl EntityInspector {
    /// Creates a new inspector with the window open by default.
    pub fn new() -> Self {
        Self {
            open: true,
            selected_archetype: None,
        }
    }

    /// Draws the inspector window.  Call once per egui frame.
    ///
    /// The window can be closed with the × button; re-open by setting
    /// [`open`](Self::open) to `true` or toggling from the overlay control bar.
    pub fn show(&mut self, ctx: &egui::Context, world: &World) {
        let mut open = self.open;
        egui::Window::new("🔍 Entity Inspector")
            .open(&mut open)
            .resizable(true)
            .default_pos([10.0, 320.0])
            .default_size([360.0, 460.0])
            .show(ctx, |ui| {
                self.draw_contents(ui, world);
            });
        self.open = open;
    }

    fn draw_contents(&mut self, ui: &mut egui::Ui, world: &World) {
        // ── header ──────────────────────────────────────────────────────────
        let total_entities: usize = world
            .archetype_iter()
            .map(|(_, _, entities)| entities.len())
            .sum();

        ui.horizontal(|ui| {
            ui.label(format!(
                "Entities: {}   Archetypes: {}",
                total_entities,
                world.archetype_count(),
            ));
            if ui.small_button("Collapse all").clicked() {
                self.selected_archetype = None;
            }
        });
        ui.separator();

        // ── archetype list ───────────────────────────────────────────────────
        egui::ScrollArea::vertical()
            .id_source("inspector_scroll")
            .show(ui, |ui| {
                for (arch_idx, type_ids, entities) in world.archetype_iter() {
                    if entities.is_empty() {
                        continue;
                    }

                    let header = archetype_label(arch_idx, type_ids, entities.len());
                    let is_open = self.selected_archetype == Some(arch_idx);

                    let response = egui::CollapsingHeader::new(&header)
                        .id_source(arch_idx)
                        .default_open(is_open)
                        .show(ui, |ui| {
                            for &entity in entities {
                                let eid = format!(
                                    "Entity #{} (gen {})",
                                    entity.id(),
                                    entity.generation()
                                );
                                egui::CollapsingHeader::new(&eid)
                                    .id_source(egui::Id::new(arch_idx).with(entity.id()))
                                    .show(ui, |ui| {
                                        draw_entity_components(ui, world, entity, type_ids);
                                    });
                            }
                        });

                    // Track which archetype the user last opened.
                    if response.header_response.clicked() {
                        self.selected_archetype = if is_open { None } else { Some(arch_idx) };
                    }
                }
            });
    }
}

impl Default for EntityInspector {
    fn default() -> Self {
        Self::new()
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Builds a human-readable label for an archetype header row.
fn archetype_label(idx: usize, type_ids: &[TypeId], entity_count: usize) -> String {
    // Map well-known TypeIds to short names.
    let names: Vec<&str> = type_ids
        .iter()
        .map(|tid| type_name_short(tid))
        .collect();

    format!(
        "Arch #{idx}  [{count} entities]  {{ {types} }}",
        count = entity_count,
        types = names.join(", ")
    )
}

/// Maps a [`TypeId`] to a short human-readable name for known types.
fn type_name_short(tid: &TypeId) -> &'static str {
    // Check known component types.
    if *tid == TypeId::of::<Transform>() {
        "Transform"
    } else if *tid == TypeId::of::<MeshHandle>() {
        "MeshHandle"
    } else if *tid == TypeId::of::<MaterialHandle>() {
        "MaterialHandle"
    } else {
        // For unknown types we show a placeholder.  The debug overlay in
        // `context.rs` can register additional type name overrides in
        // a future version.
        "?"
    }
}

/// Draws the component values for a single entity inside the inspector.
///
/// Currently handles: `Transform`, `MeshHandle`, `MaterialHandle`.
/// Unknown component types are shown as "? (unknown type)".
fn draw_entity_components(
    ui: &mut egui::Ui,
    world: &World,
    entity: redox_ecs::Entity,
    type_ids: &[TypeId],
) {
    for tid in type_ids {
        if *tid == TypeId::of::<Transform>() {
            if let Some(tf) = world.get_component::<Transform>(entity) {
                egui::CollapsingHeader::new("Transform")
                    .id_source(egui::Id::new(entity.id()).with("transform"))
                    .default_open(true)
                    .show(ui, |ui| {
                        draw_transform(ui, tf);
                    });
            }
        } else if *tid == TypeId::of::<MeshHandle>() {
            if let Some(mh) = world.get_component::<MeshHandle>(entity) {
                ui.label(format!("MeshHandle: {:?}", mh.0.id()));
            }
        } else if *tid == TypeId::of::<MaterialHandle>() {
            if let Some(mat) = world.get_component::<MaterialHandle>(entity) {
                ui.label(format!("MaterialHandle: {:?}", mat.0.id()));
            }
        } else {
            ui.label(
                egui::RichText::new(format!("? (TypeId {:?})", tid))
                    .weak()
                    .italics(),
            );
        }
    }
}

/// Draws the fields of a [`Transform`] component in a compact grid.
fn draw_transform(ui: &mut egui::Ui, tf: &Transform) {
    egui::Grid::new("transform_grid")
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            let t = tf.translation;
            ui.label("translation");
            ui.label(format!("({:.2}, {:.2}, {:.2})", t.x, t.y, t.z));
            ui.end_row();

            // Convert quaternion to Euler angles (degrees) for readability.
            let (rx, ry, rz) = quat_to_euler_deg(tf.rotation);
            ui.label("rotation (°)");
            ui.label(format!("({:.1}, {:.1}, {:.1})", rx, ry, rz));
            ui.end_row();

            let s = tf.scale;
            ui.label("scale");
            ui.label(format!("({:.3}, {:.3}, {:.3})", s.x, s.y, s.z));
            ui.end_row();
        });
}

/// Converts a [`glam::Quat`] to ZYX Euler angles in **degrees**.
fn quat_to_euler_deg(q: redox_math::Quat) -> (f32, f32, f32) {
    let (rx, ry, rz) = q.to_euler(glam::EulerRot::ZYX);
    (rx.to_degrees(), ry.to_degrees(), rz.to_degrees())
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_new() {
        let inspector = EntityInspector::new();
        assert!(inspector.open);
        assert_eq!(inspector.selected_archetype, None);
    }

    #[test]
    fn archetype_label_format() {
        let label = archetype_label(0, &[], 5);
        assert!(label.contains("Arch #0"));
        assert!(label.contains("5 entities"));
    }
}

use redox_ecs::{Entity, World};
use redox_editor_core::{UndoCommand, UndoStack};
use redox_gameplay::AttributeSet;
use redox_reflect::{ReflectValue, TypeRegistry};

pub struct InspectorPanel;

impl InspectorPanel {
    pub fn show(
        ui: &mut egui::Ui,
        registry: &TypeRegistry,
        entity: Option<Entity>,
        world: &mut World,
        undo: &mut UndoStack,
    ) {
        let Some(e) = entity else {
            ui.label("No selection");
            return;
        };
        ui.label(format!("📦 Entity: #{}", e.id()));

        if let Some(attrs) = world.get_component::<AttributeSet>(e) {
            ui.label(format!(
                "GAS  HP {:.0}/{:.0}  STA {:.0}/{:.0}",
                attrs.health.current, attrs.health.max, attrs.stamina.current, attrs.stamina.max
            ));
        }

        let type_names: Vec<&'static str> = registry.type_names().collect();
        for name in type_names {
            let Some(mut val) = registry.read_component(world, e, name) else {
                continue;
            };
            let before = val.clone();
            let mut changed = false;
            ui.collapsing(name, |ui| {
                changed = Self::edit_value(ui, &mut val);
            });
            if changed {
                let _ = registry.insert_component(world, e, &val);
                undo.push(UndoCommand::SetComponent {
                    entity: e,
                    type_name: name.to_string(),
                    before: Some(before),
                    after: Some(val),
                });
            }
        }

        ui.separator();
        ui.menu_button("➕ Add Component", |ui| {
            for name in registry.type_names() {
                if registry.read_component(world, e, name).is_some() {
                    continue;
                }
                if ui.button(name).clicked() {
                    // Spawn default via insert of empty-ish struct from schema isn't available —
                    // Transform already present typically. Skip unregistered defaults.
                    ui.close_menu();
                }
            }
        });
    }

    fn edit_value(ui: &mut egui::Ui, val: &mut ReflectValue) -> bool {
        match val {
            ReflectValue::Struct { fields, .. } => {
                let mut changed = false;
                for (k, v) in fields.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.label(k.as_str());
                        if Self::edit_value(ui, v) {
                            changed = true;
                        }
                    });
                }
                changed
            }
            ReflectValue::F64(f) => {
                let mut v = *f as f32;
                let r = ui.add(egui::DragValue::new(&mut v).speed(0.05));
                if r.changed() {
                    *f = v as f64;
                    true
                } else {
                    false
                }
            }
            ReflectValue::I64(i) => {
                let r = ui.add(egui::DragValue::new(i));
                r.changed()
            }
            ReflectValue::Bool(b) => ui.checkbox(b, "").changed(),
            ReflectValue::String(s) => ui.text_edit_singleline(s).changed(),
            ReflectValue::List(items) => {
                let mut changed = false;
                if items.len() == 3 {
                    // Vec3: X Y Z
                    let labels = ["X", "Y", "Z"];
                    ui.horizontal(|ui| {
                        for (i, item) in items.iter_mut().enumerate() {
                            ui.label(labels[i]);
                            if let ReflectValue::F64(f) = item {
                                let mut v = *f as f32;
                                if ui.add(egui::DragValue::new(&mut v).speed(0.05)).changed() {
                                    *f = v as f64;
                                    changed = true;
                                }
                            }
                        }
                    });
                } else if items.len() == 4 {
                    let labels = ["X", "Y", "Z", "W"];
                    ui.horizontal(|ui| {
                        for (i, item) in items.iter_mut().enumerate() {
                            ui.label(labels[i]);
                            if let ReflectValue::F64(f) = item {
                                let mut v = *f as f32;
                                if ui.add(egui::DragValue::new(&mut v).speed(0.01)).changed() {
                                    *f = v as f64;
                                    changed = true;
                                }
                            }
                        }
                    });
                } else {
                    for it in items.iter_mut() {
                        if Self::edit_value(ui, it) {
                            changed = true;
                        }
                    }
                }
                changed
            }
            ReflectValue::Null => {
                ui.label("Null");
                false
            }
        }
    }
}

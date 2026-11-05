//! Value widgets of the inspector. Each one returns the new value only when the user changed it.

use egui::Ui;
use redox_editor_core::inspector_model::Row;
use redox_reflect::ReflectValue;
use redox_reflect::meta::WidgetHint;

/// Draws the editable value of `row` and returns the new value when the user changed it.
///
/// The widget comes from the field's [`WidgetHint`] when it asks for one the value fits, and
/// from the shape of the value otherwise.
pub fn value_widget(ui: &mut Ui, row: &Row, value: &ReflectValue) -> Option<ReflectValue> {
    match (row.widget, value) {
        (WidgetHint::Color, ReflectValue::List(items)) if items.len() == 3 => {
            color_widget(ui, items)
        }
        (WidgetHint::Angle, ReflectValue::F64(_) | ReflectValue::I64(_)) => {
            angle_widget(ui, value)
        }
        (WidgetHint::Slider, ReflectValue::F64(_) | ReflectValue::I64(_)) => {
            slider_widget(ui, row, value)
        }
        (WidgetHint::Multiline, ReflectValue::String(text)) => multiline_widget(ui, text),
        (WidgetHint::Enum(options), ReflectValue::String(text)) => {
            enum_widget(ui, row, options, text)
        }
        (_, ReflectValue::F64(_) | ReflectValue::I64(_)) => number_widget(ui, row, value),
        (_, ReflectValue::Bool(flag)) => checkbox_widget(ui, *flag),
        (_, ReflectValue::String(text)) => text_widget(ui, text),
        (_, ReflectValue::List(items)) => vector_widget(ui, row, items),
        (_, ReflectValue::Null | ReflectValue::Struct { .. }) => None,
    }
}

/// Drag step of a number: the field step, or a small default.
fn speed(row: &Row) -> f64 {
    row.step.unwrap_or(0.05)
}

fn number_widget(ui: &mut Ui, row: &Row, value: &ReflectValue) -> Option<ReflectValue> {
    match value {
        ReflectValue::I64(current) => {
            let mut number = *current;
            let mut drag = egui::DragValue::new(&mut number);
            if let Some((min, max)) = row.range {
                drag = drag.range(min as i64..=max as i64);
            }
            if let Some(unit) = row.unit {
                drag = drag.suffix(format!(" {unit}"));
            }
            let changed = ui.add(drag).changed();
            changed.then_some(ReflectValue::I64(number))
        }
        ReflectValue::F64(current) => {
            let mut number = *current;
            let mut drag = egui::DragValue::new(&mut number).speed(speed(row));
            if let Some((min, max)) = row.range {
                drag = drag.range(min..=max);
            }
            if let Some(unit) = row.unit {
                drag = drag.suffix(format!(" {unit}"));
            }
            let changed = ui.add(drag).changed();
            changed.then_some(ReflectValue::F64(number))
        }
        _ => None,
    }
}

/// Radians shown and typed as degrees.
fn angle_widget(ui: &mut Ui, value: &ReflectValue) -> Option<ReflectValue> {
    let radians = f64::from(value.as_f32()?);
    let mut degrees = radians.to_degrees();
    let drag = egui::DragValue::new(&mut degrees).speed(0.5).suffix("°");
    let changed = ui.add(drag).changed();
    changed.then(|| ReflectValue::F64(degrees.to_radians()))
}

fn slider_widget(ui: &mut Ui, row: &Row, value: &ReflectValue) -> Option<ReflectValue> {
    let Some((min, max)) = row.range else {
        return number_widget(ui, row, value);
    };
    let mut number = f64::from(value.as_f32()?);
    let mut slider = egui::Slider::new(&mut number, min..=max);
    if let Some(unit) = row.unit {
        slider = slider.suffix(format!(" {unit}"));
    }
    let changed = ui.add(slider).changed();
    changed.then_some(ReflectValue::F64(number))
}

fn color_widget(ui: &mut Ui, items: &[ReflectValue]) -> Option<ReflectValue> {
    let mut rgb = [0.0_f32; 3];
    for (slot, item) in rgb.iter_mut().zip(items) {
        *slot = item.as_f32()?;
    }
    let changed = ui.color_edit_button_rgb(&mut rgb).changed();
    changed.then(|| ReflectValue::vec3(rgb[0], rgb[1], rgb[2]))
}

/// Two to four numbers in one row, with axis labels.
fn vector_widget(ui: &mut Ui, row: &Row, items: &[ReflectValue]) -> Option<ReflectValue> {
    const AXES: [&str; 4] = ["X", "Y", "Z", "W"];
    let mut numbers: Vec<f64> = items
        .iter()
        .map(|item| item.as_f32().map(f64::from))
        .collect::<Option<Vec<f64>>>()?;
    let mut changed = false;
    ui.horizontal(|ui| {
        for (index, number) in numbers.iter_mut().enumerate() {
            ui.label(AXES.get(index).copied().unwrap_or("·"));
            let drag = egui::DragValue::new(number).speed(speed(row));
            changed |= ui.add(drag).changed();
        }
    });
    changed.then(|| ReflectValue::List(numbers.into_iter().map(ReflectValue::F64).collect()))
}

fn checkbox_widget(ui: &mut Ui, flag: bool) -> Option<ReflectValue> {
    let mut value = flag;
    let changed = ui.checkbox(&mut value, "").changed();
    changed.then_some(ReflectValue::Bool(value))
}

fn text_widget(ui: &mut Ui, text: &str) -> Option<ReflectValue> {
    let mut edited = text.to_string();
    let changed = ui.text_edit_singleline(&mut edited).changed();
    changed.then_some(ReflectValue::String(edited))
}

fn multiline_widget(ui: &mut Ui, text: &str) -> Option<ReflectValue> {
    let mut edited = text.to_string();
    let changed = ui
        .add(egui::TextEdit::multiline(&mut edited).desired_rows(3))
        .changed();
    changed.then_some(ReflectValue::String(edited))
}

/// Drop-down list of fixed options.
fn enum_widget(
    ui: &mut Ui,
    row: &Row,
    options: &[&str],
    current: &str,
) -> Option<ReflectValue> {
    let mut picked: Option<&str> = None;
    egui::ComboBox::from_id_salt((row.component.as_str(), row.field))
        .selected_text(current)
        .show_ui(ui, |ui| {
            for option in options {
                if ui
                    .selectable_label(*option == current, *option)
                    .clicked()
                {
                    picked = Some(*option);
                }
            }
        });
    picked.map(|option| ReflectValue::String(option.to_string()))
}

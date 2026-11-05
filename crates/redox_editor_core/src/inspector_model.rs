//! Inspector state without egui: which components and fields the selected entities show, and
//! the undo commands that an edit produces.
//!
//! The panel in `redox_editor_ui` draws an [`InspectorModel`] and sends edits back through its
//! methods. The methods return commands and do not apply them: the caller applies them and
//! records them as one undo step.

mod components;
mod edit;
mod search;

use redox_ecs::{Entity, World};
use redox_reflect::meta::{FieldError, FieldMeta, WidgetHint};
use redox_reflect::{ReflectSchema, ReflectValue, TypeRegistry};

use crate::undo::UndoCommand;

pub use components::{AddOutcome, AddableType, RemoveOutcome, addable_types};
pub use edit::{copy_component, paste_component_values};

/// The value a row shows: one value that all selected entities share, or a mixed state.
#[derive(Clone, Debug, PartialEq)]
pub enum RowValue {
    /// All selected entities hold this value.
    Single(ReflectValue),
    /// The selected entities hold different values.
    Mixed,
}

/// One editable field of a component.
#[derive(Clone, Debug)]
pub struct Row {
    /// Registered name of the component.
    pub component: String,
    /// Field name as stored in the reflected value.
    pub field: &'static str,
    /// Label to show (the field name when no label is set).
    pub label: String,
    /// Category of the field, if any.
    pub category: Option<&'static str>,
    /// Shown value.
    pub value: RowValue,
    /// `true` when `value` is [`RowValue::Mixed`].
    pub mixed: bool,
    /// Hint shown on hover.
    pub tooltip: Option<&'static str>,
    /// Unit shown after the value.
    pub unit: Option<&'static str>,
    /// Smallest and largest allowed number.
    pub range: Option<(f64, f64)>,
    /// Grid for rounding numbers.
    pub step: Option<f64>,
    /// Widget the field asks for.
    pub widget: WidgetHint,
    /// The field cannot be edited.
    pub read_only: bool,
    /// The value differs from the field default (metadata default, or the type's `Default`).
    pub modified_from_default: bool,
    /// File extensions an asset reference may point to; empty means any file.
    pub accepts: &'static [&'static str],
}

/// The rows of one component on the selected entities.
#[derive(Clone, Debug)]
pub struct Section {
    /// Registered name of the component.
    pub component: String,
    /// Category of the component, if any.
    pub category: Option<&'static str>,
    /// Rows in declaration order.
    pub rows: Vec<Row>,
}

/// What the inspector shows for the current selection.
#[derive(Clone, Debug, Default)]
pub struct InspectorModel {
    /// The selected entities, in the order given to [`InspectorModel::build`].
    pub entities: Vec<Entity>,
    /// Components that every selected entity has, sorted by category and then by name.
    pub sections: Vec<Section>,
    /// Components that only some of the selected entities have. They are not shown as sections.
    pub partial_components: Vec<String>,
}

impl InspectorModel {
    /// Builds the model for `entities`.
    ///
    /// Only components present on every entity become sections. Hidden, editor-only and (unless
    /// `show_advanced`) advanced fields are left out. `filter` is a case-insensitive substring
    /// matched against field name, label, category and tooltip; a section with no matching rows
    /// is left out when the filter is not empty.
    pub fn build(
        registry: &TypeRegistry,
        world: &World,
        entities: &[Entity],
        filter: &str,
        show_advanced: bool,
    ) -> Self {
        let needle = filter.trim().to_lowercase();
        let mut names: Vec<&str> = registry.type_names().collect();
        names.sort_unstable();

        let mut sections = Vec::new();
        let mut partial_components = Vec::new();
        for name in names {
            let stored: Vec<Option<ReflectValue>> = entities
                .iter()
                .map(|&entity| registry.read_component(world, entity, name))
                .collect();
            let present = stored.iter().filter(|value| value.is_some()).count();
            if present == 0 {
                continue;
            }
            if present < entities.len() {
                partial_components.push(name.to_string());
                continue;
            }
            let Some(schema) = registry.schema(name) else {
                continue;
            };
            let values: Vec<ReflectValue> = stored.into_iter().flatten().collect();
            let rows = rows_for(registry, schema, &values, &needle, show_advanced);
            if rows.is_empty() && !needle.is_empty() {
                continue;
            }
            sections.push(Section {
                component: name.to_string(),
                category: schema.meta.category,
                rows,
            });
        }
        sections.sort_by(|a, b| {
            (a.category.is_none(), a.category.unwrap_or(""))
                .cmp(&(b.category.is_none(), b.category.unwrap_or("")))
                .then_with(|| a.component.cmp(&b.component))
        });

        Self {
            entities: entities.to_vec(),
            sections,
            partial_components,
        }
    }

    /// Commands that set `field` of `component` to `value` on every selected entity, one command
    /// per entity. The value is checked against the field metadata first (read-only, type shape,
    /// range, step).
    pub fn edit(
        &self,
        registry: &TypeRegistry,
        world: &World,
        component: &str,
        field: &str,
        value: ReflectValue,
    ) -> Result<Vec<UndoCommand>, FieldError> {
        edit::field_commands(registry, world, &self.entities, component, field, &value)
    }

    /// Commands that restore one field to its default: the metadata `default`, or the field of the
    /// type's `Default` value when there is none.
    pub fn reset_field(
        &self,
        registry: &TypeRegistry,
        world: &World,
        component: &str,
        field: &str,
    ) -> Result<Vec<UndoCommand>, FieldError> {
        edit::reset_field_commands(registry, world, &self.entities, component, field)
    }

    /// Commands that point the asset reference `field` of `component` at `path`. The file extension
    /// must be one of the field's `accepts`; otherwise the result is [`FieldError::TypeMismatch`]
    /// and nothing changes. The path is stored as the reference value.
    pub fn assign_asset_path(
        &self,
        registry: &TypeRegistry,
        world: &World,
        component: &str,
        field: &str,
        path: &str,
    ) -> Result<Vec<UndoCommand>, FieldError> {
        edit::asset_path_commands(registry, world, &self.entities, component, field, path)
    }

    /// Commands that restore every editable field of `component` to its default. Read-only fields
    /// keep their current value.
    pub fn reset_component(
        &self,
        registry: &TypeRegistry,
        world: &World,
        component: &str,
    ) -> Result<Vec<UndoCommand>, FieldError> {
        edit::reset_component_commands(registry, world, &self.entities, component)
    }
}

/// Rows of one component, built from its value on every selected entity (`values` has one
/// entry per entity).
fn rows_for(
    registry: &TypeRegistry,
    schema: &ReflectSchema,
    values: &[ReflectValue],
    needle: &str,
    show_advanced: bool,
) -> Vec<Row> {
    let type_default = registry.default_value(schema.type_name);
    schema
        .fields
        .iter()
        .filter(|field| is_listed(&field.meta, show_advanced))
        .filter(|field| needle.is_empty() || search::field_matches(field, needle))
        .filter_map(|field| {
            let stored: Vec<&ReflectValue> = values
                .iter()
                .filter_map(|value| value.field(field.name))
                .collect();
            if stored.is_empty() || stored.len() != values.len() {
                return None;
            }
            let meta = &field.meta;
            let mixed = stored.iter().any(|value| *value != stored[0]);
            let value = if mixed {
                RowValue::Mixed
            } else {
                RowValue::Single(stored[0].clone())
            };
            let modified_from_default = match &value {
                RowValue::Single(current) => {
                    let reference = meta.default.clone().or_else(|| {
                        type_default
                            .as_ref()
                            .and_then(|default| default.field(field.name).cloned())
                    });
                    reference.is_some_and(|reference| reference != *current)
                }
                RowValue::Mixed => false,
            };
            Some(Row {
                component: schema.type_name.to_string(),
                field: field.name,
                label: meta.label.unwrap_or(field.name).to_string(),
                category: meta.category,
                value,
                mixed,
                tooltip: meta.tooltip,
                unit: meta.unit,
                range: meta.range,
                step: meta.step,
                widget: meta.widget,
                read_only: meta.read_only,
                modified_from_default,
                accepts: meta.accepts,
            })
        })
        .collect()
}

/// Whether a field is shown at all: hidden and editor-only fields never are, advanced fields only
/// when `show_advanced` is set.
fn is_listed(meta: &FieldMeta, show_advanced: bool) -> bool {
    !(meta.hidden || meta.editor_only || (meta.advanced && !show_advanced))
}

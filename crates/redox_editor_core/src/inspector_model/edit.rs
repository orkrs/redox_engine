//! Edits of component values. Every edit becomes `SetComponent` commands, one per entity; the
//! caller applies them and records them as one undo step.

use redox_ecs::{Entity, World};
use redox_reflect::meta::FieldError;
use redox_reflect::{ReflectSchema, ReflectValue, TypeRegistry};

use crate::guid::GuidResolver;
use crate::undo::UndoCommand;
use redox_scene::EntityGuid;

/// Commands that set `field` of `component` to `value` on every entity. The value is checked
/// against the field metadata first.
pub(super) fn field_commands(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    component: &str,
    field: &str,
    value: &ReflectValue,
) -> Result<Vec<UndoCommand>, FieldError> {
    let schema = schema_of(registry, component)?;
    let definition = schema
        .fields
        .iter()
        .find(|f| f.name == field)
        .ok_or_else(|| FieldError::UnknownField(field.to_string()))?;
    let checked = definition.validate(value)?;
    // Entities without the component are skipped: a mixed selection edits those that have it.
    let mut commands = Vec::new();
    for &entity in entities {
        let Some(before) = registry.read_component(world, entity, component) else {
            continue;
        };
        let after = with_field(&before, field, checked.clone())?;
        commands.push(set_command(world, entity, component, before, after)?);
    }
    Ok(commands)
}

/// Commands that point the asset reference `field` at `path`, after checking the extension
/// against the field's `accepts`.
pub(super) fn asset_path_commands(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    component: &str,
    field: &str,
    path: &str,
) -> Result<Vec<UndoCommand>, FieldError> {
    let schema = schema_of(registry, component)?;
    let definition = schema
        .fields
        .iter()
        .find(|f| f.name == field)
        .ok_or_else(|| FieldError::UnknownField(field.to_string()))?;
    if !extension_accepted(path, definition.meta.accepts) {
        return Err(FieldError::TypeMismatch {
            expected: definition.meta.accepts.join(", "),
        });
    }
    let value = ReflectValue::handle(definition.type_name, Some(path.to_string()));
    field_commands(registry, world, entities, component, field, &value)
}

/// Whether the extension of `path` is in `accepts` (case-insensitive, leading dot optional).
/// An empty list accepts every path.
fn extension_accepted(path: &str, accepts: &[&str]) -> bool {
    if accepts.is_empty() {
        return true;
    }
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension {
        Some(extension) => accepts.iter().any(|accepted| {
            accepted
                .trim_start_matches('.')
                .eq_ignore_ascii_case(&extension)
        }),
        None => false,
    }
}

/// Commands that put `field` back to its default value on every entity.
pub(super) fn reset_field_commands(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    component: &str,
    field: &str,
) -> Result<Vec<UndoCommand>, FieldError> {
    let schema = schema_of(registry, component)?;
    let definition = schema
        .fields
        .iter()
        .find(|f| f.name == field)
        .ok_or_else(|| FieldError::UnknownField(field.to_string()))?;
    let default = definition
        .meta
        .default
        .clone()
        .or_else(|| type_default_field(registry, component, field))
        .ok_or_else(|| FieldError::UnknownField(field.to_string()))?;
    field_commands(registry, world, entities, component, field, &default)
}

/// Commands that restore every editable field of `component` on every entity.
pub(super) fn reset_component_commands(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    component: &str,
) -> Result<Vec<UndoCommand>, FieldError> {
    let schema = schema_of(registry, component)?;
    let base = registry
        .default_value(component)
        .ok_or_else(|| FieldError::UnknownType(component.to_string()))?;
    let mut commands = Vec::new();
    for &entity in entities {
        let Some(before) = registry.read_component(world, entity, component) else {
            continue;
        };
        let after = reset_value(&base, &before, schema)?;
        commands.push(set_command(world, entity, component, before, after)?);
    }
    Ok(commands)
}

/// Copies the current value of `component` on `entity`, for pasting onto other entities.
pub fn copy_component(
    registry: &TypeRegistry,
    world: &World,
    entity: Entity,
    component: &str,
) -> Option<ReflectValue> {
    registry.read_component(world, entity, component)
}

/// Commands that paste the fields of `source` into `target` on every entity that has `target`.
///
/// Only fields with the same name and the same type in both schemas are copied, and only when
/// the pasted value passes the target field's metadata. Other fields are ignored.
pub fn paste_component_values(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    target: &str,
    source: &ReflectValue,
) -> Result<Vec<UndoCommand>, FieldError> {
    let ReflectValue::Struct {
        type_name: source_type,
        fields: source_fields,
    } = source
    else {
        return Err(FieldError::Reflect(
            "source is not a component value".to_string(),
        ));
    };
    let source_schema = schema_of(registry, source_type)?;
    let target_schema = schema_of(registry, target)?;
    let pasted: Vec<(&'static str, ReflectValue)> = target_schema
        .fields
        .iter()
        .filter_map(|field| {
            let counterpart = source_schema
                .fields
                .iter()
                .find(|s| s.name == field.name && s.type_name == field.type_name)?;
            let (_, value) = source_fields
                .iter()
                .find(|(name, _)| name == counterpart.name)?;
            let checked = field.validate(value).ok()?;
            Some((field.name, checked))
        })
        .collect();

    let mut commands = Vec::new();
    for &entity in entities {
        let Some(before) = registry.read_component(world, entity, target) else {
            continue;
        };
        let mut after = before.clone();
        for (name, value) in &pasted {
            after = with_field(&after, name, value.clone())?;
        }
        if after != before {
            commands.push(set_command(world, entity, target, before, after)?);
        }
    }
    Ok(commands)
}

fn schema_of<'a>(
    registry: &'a TypeRegistry,
    component: &str,
) -> Result<&'a ReflectSchema, FieldError> {
    registry
        .schema(component)
        .ok_or_else(|| FieldError::UnknownType(component.to_string()))
}

/// The guid of `entity`: commands address entities by guid, so an entity without one cannot be
/// edited until the caller gives it one (`GuidResolver::ensure`).
pub(super) fn guid_for(world: &World, entity: Entity) -> Result<EntityGuid, FieldError> {
    GuidResolver::guid_of(world, entity)
        .ok_or_else(|| FieldError::MissingComponent("EntityGuid".to_string()))
}

fn set_command(
    world: &World,
    entity: Entity,
    component: &str,
    before: ReflectValue,
    after: ReflectValue,
) -> Result<UndoCommand, FieldError> {
    Ok(UndoCommand::SetComponent {
        guid: guid_for(world, entity)?,
        type_name: component.to_string(),
        before: Some(before),
        after: Some(after),
    })
}

fn type_default_field(
    registry: &TypeRegistry,
    component: &str,
    field: &str,
) -> Option<ReflectValue> {
    registry.default_value(component)?.field(field).cloned()
}

/// `value` with one field replaced; the field must exist in the struct.
fn with_field(
    value: &ReflectValue,
    field: &str,
    new_value: ReflectValue,
) -> Result<ReflectValue, FieldError> {
    let ReflectValue::Struct { type_name, fields } = value else {
        return Err(FieldError::Reflect(
            "component value is not a struct".to_string(),
        ));
    };
    if !fields.iter().any(|(name, _)| name == field) {
        return Err(FieldError::UnknownField(field.to_string()));
    }
    let fields = fields
        .iter()
        .map(|(name, old)| {
            if name == field {
                (name.clone(), new_value.clone())
            } else {
                (name.clone(), old.clone())
            }
        })
        .collect();
    Ok(ReflectValue::Struct {
        type_name: type_name.clone(),
        fields,
    })
}

/// `before` with every editable field replaced by its default. Read-only fields keep their value.
fn reset_value(
    base: &ReflectValue,
    before: &ReflectValue,
    schema: &ReflectSchema,
) -> Result<ReflectValue, FieldError> {
    let mut value = before.clone();
    for field in schema.fields.iter().filter(|f| !f.meta.read_only) {
        let fresh = field
            .meta
            .default
            .clone()
            .or_else(|| base.field(field.name).cloned());
        if let Some(fresh) = fresh {
            value = with_field(&value, field.name, fresh)?;
        }
    }
    Ok(value)
}

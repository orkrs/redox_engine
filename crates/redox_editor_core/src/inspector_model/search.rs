//! Case-insensitive substring search over field names and field metadata.

use redox_reflect::ReflectField;

/// Whether `field` matches `needle`, a lower-case and non-empty search text. The name, label,
/// category and tooltip of the field are searched.
pub(super) fn field_matches(field: &ReflectField, needle: &str) -> bool {
    let hit = |text: &str| text.to_lowercase().contains(needle);
    let meta = &field.meta;
    hit(field.name)
        || meta.label.is_some_and(&hit)
        || meta.category.is_some_and(&hit)
        || meta.tooltip.is_some_and(hit)
}

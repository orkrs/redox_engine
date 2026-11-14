//! Catalog of component types for the Add Component menu: categories, tooltips, search, and
//! soft dependency warnings.
//!
//! Hard dependencies come from `#[reflect(requires = "...")]` and are applied automatically when
//! adding a component ([`crate::inspector_model::InspectorModel::add_component`]). Soft
//! prerequisites are editor recommendations only (for example physics colliders without a rigid
//! body); they trigger a Continue/Cancel prompt via [`dependency_warning_for_add`].

use redox_ecs::{Entity, World};
use redox_reflect::{ReflectSchema, TypeRegistry};

/// One row in the component catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Registered name of the component type.
    pub type_name: &'static str,
    /// Inspector category, if any.
    pub category: Option<&'static str>,
    /// Hint shown in the add menu.
    pub tooltip: Option<&'static str>,
}

/// Types offered by the add menu (same shape as [`CatalogEntry`]).
pub type AddableType = CatalogEntry;

/// Lists every registered type that may appear in the catalog.
#[derive(Clone, Copy, Debug, Default)]
pub struct ComponentCatalog;

impl ComponentCatalog {
    /// Every type in `registry` that is visible in the catalog, sorted by category then name.
    pub fn entries(registry: &TypeRegistry, include_editor_only: bool) -> Vec<CatalogEntry> {
        let mut out: Vec<CatalogEntry> = registry
            .type_names()
            .filter_map(|name| entry_of(registry, name, include_editor_only))
            .collect();
        sort_entries(&mut out);
        out
    }

    /// `entries` filtered by a case-insensitive substring of the name, category or tooltip.
    pub fn search(
        registry: &TypeRegistry,
        filter: &str,
        include_editor_only: bool,
    ) -> Vec<CatalogEntry> {
        let needle = filter.trim().to_lowercase();
        let mut out: Vec<CatalogEntry> = registry
            .type_names()
            .filter_map(|name| {
                let entry = entry_of(registry, name, include_editor_only)?;
                if needle.is_empty() {
                    return Some(entry);
                }
                let matches = name.to_lowercase().contains(&needle)
                    || entry
                        .category
                        .is_some_and(|c| c.to_lowercase().contains(&needle))
                    || entry
                        .tooltip
                        .is_some_and(|t| t.to_lowercase().contains(&needle));
                matches.then_some(entry)
            })
            .collect();
        sort_entries(&mut out);
        out
    }

    /// Types that may be added to `entities`: not present on every selected entity, matching
    /// `filter`. Editor-only types are included only when `include_editor_only` is set.
    pub fn addable(
        registry: &TypeRegistry,
        world: &World,
        entities: &[Entity],
        filter: &str,
        include_editor_only: bool,
    ) -> Vec<CatalogEntry> {
        let needle = filter.trim().to_lowercase();
        let mut out: Vec<CatalogEntry> = registry
            .type_names()
            .filter_map(|name| {
                let entry = entry_of(registry, name, include_editor_only)?;
                let present_everywhere = !entities.is_empty()
                    && entities
                        .iter()
                        .all(|&entity| registry.read_component(world, entity, name).is_some());
                if present_everywhere {
                    return None;
                }
                let matches = needle.is_empty()
                    || name.to_lowercase().contains(&needle)
                    || entry
                        .category
                        .is_some_and(|c| c.to_lowercase().contains(&needle))
                    || entry
                        .tooltip
                        .is_some_and(|t| t.to_lowercase().contains(&needle));
                matches.then_some(entry)
            })
            .collect();
        sort_entries(&mut out);
        out
    }

    /// Recommended components that are not auto-added via `requires` metadata.
    pub fn soft_prerequisites(type_name: &str) -> &'static [&'static str] {
        match type_name {
            "Collider" => &["RigidBody"],
            _ => &[],
        }
    }
}

/// Same as [`ComponentCatalog::addable`]; kept for existing call sites.
pub fn addable_types(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    filter: &str,
    include_editor_only: bool,
) -> Vec<AddableType> {
    ComponentCatalog::addable(registry, world, entities, filter, include_editor_only)
}

fn entry_of(
    registry: &TypeRegistry,
    name: &str,
    include_editor_only: bool,
) -> Option<CatalogEntry> {
    let schema = registry.schema(name)?;
    if !listed_in_catalog(schema, include_editor_only) {
        return None;
    }
    Some(CatalogEntry {
        type_name: schema.type_name,
        category: schema.meta.category,
        tooltip: schema.meta.tooltip,
    })
}

fn listed_in_catalog(schema: &ReflectSchema, include_editor_only: bool) -> bool {
    if schema.meta.editor_only && !include_editor_only {
        return false;
    }
    !catalog_hidden(schema)
}

/// Types whose every field is hidden or editor-only are not offered in the add menu.
fn catalog_hidden(schema: &ReflectSchema) -> bool {
    !schema.fields.is_empty()
        && schema
            .fields
            .iter()
            .all(|field| field.meta.hidden || field.meta.editor_only)
}

fn sort_entries(entries: &mut [CatalogEntry]) {
    entries.sort_by(|a, b| {
        (a.category.is_none(), a.category.unwrap_or(""))
            .cmp(&(b.category.is_none(), b.category.unwrap_or("")))
            .then_with(|| a.type_name.cmp(b.type_name))
    });
}

/// Warning shown before adding a component when soft prerequisites are missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyWarning {
    /// Component the user asked to add.
    pub type_name: String,
    /// Missing recommended types (union across the selection).
    pub missing: Vec<String>,
    /// Text for the dialog or console.
    pub message: String,
}

/// Whether an add operation should ask the user before proceeding.
pub fn dependency_warning_for_add(
    registry: &TypeRegistry,
    world: &World,
    entities: &[Entity],
    type_name: &str,
) -> Option<DependencyWarning> {
    let soft = ComponentCatalog::soft_prerequisites(type_name);
    if soft.is_empty() {
        return None;
    }
    let mut missing: Vec<String> = soft
        .iter()
        .filter(|&&required| {
            entities.iter().any(|&entity| {
                registry
                    .read_component(world, entity, required)
                    .is_none()
            })
        })
        .map(|s| (*s).to_string())
        .collect();
    if missing.is_empty() {
        return None;
    }
    missing.sort();
    missing.dedup();
    let list = missing.join(", ");
    Some(DependencyWarning {
        type_name: type_name.to_string(),
        message: format!(
            "{type_name} usually needs {list} on the same entity. Continue without adding {list}?"
        ),
        missing,
    })
}

/// Answer to the missing-dependency prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencyPromptAnswer {
    /// Add the component anyway; warnings go to the log.
    Continue,
    /// Do not add the component.
    Cancel,
}

/// What the host does after the user answers the dependency prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencyPromptOutcome {
    /// Proceed with the add (log warnings).
    Proceed,
    /// Drop the add.
    Abort,
}

/// Maps the prompt answer to the next host action.
pub fn resolve_dependency_prompt(answer: DependencyPromptAnswer) -> DependencyPromptOutcome {
    match answer {
        DependencyPromptAnswer::Continue => DependencyPromptOutcome::Proceed,
        DependencyPromptAnswer::Cancel => DependencyPromptOutcome::Abort,
    }
}
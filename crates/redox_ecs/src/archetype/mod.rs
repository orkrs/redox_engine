/// Dense, type-erased storage for one component type.
pub mod column;
/// A table of columns: one row per entity of an archetype.
pub mod table;

use crate::component::ComponentInfo;
use std::any::TypeId;
pub use table::Table;

/// A group of entities that share the exact same set of component types.
///
/// Each archetype has a `Table` for storing component data. `types` doubles as
/// the cached sorted archetype key used by `World::type_to_archetype` lookups.
pub struct Archetype {
    #[allow(dead_code)]
    pub(crate) id: usize,
    pub(crate) types: Vec<TypeId>,
    pub(crate) table: Table,
    pub(crate) component_infos: Vec<ComponentInfo>,
}

// `Archetype` is `Send + Sync` automatically via `Table`.

impl Archetype {
    /// Creates a new archetype with the given component infos.
    pub fn new(id: usize, component_infos: Vec<ComponentInfo>) -> Self {
        let mut types: Vec<_> = component_infos.iter().map(|i| i.type_id).collect();
        types.sort();

        Self {
            id,
            types,
            table: Table::new(&component_infos),
            component_infos,
        }
    }

    /// Returns the sorted set of component [`TypeId`]s that define this archetype.
    pub fn component_types(&self) -> &[TypeId] {
        &self.types
    }

    /// Returns the entities stored in this archetype.
    pub fn entities(&self) -> &[crate::entity::Entity] {
        self.table.entities()
    }
}

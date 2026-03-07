pub mod column;
pub mod edges;
pub mod table;

use crate::component::ComponentInfo;
pub use edges::Edges;
use std::any::TypeId;
pub use table::Table;

/// A group of entities that share the exact same set of component types.
///
/// Each archetype has a `Table` for storing component data and `Edges` for
/// transitions to other archetypes.
pub struct Archetype {
    #[allow(dead_code)]
    pub(crate) id: usize,
    #[allow(dead_code)]
    pub(crate) types: Vec<TypeId>,
    pub(crate) table: Table,
    #[allow(dead_code)]
    pub(crate) edges: Edges,
    pub(crate) component_infos: Vec<ComponentInfo>,
}

// SAFETY: Archetype contains only data that can be safely shared across threads.
// All fields are either primitives, Vec, or Table/Edges which we also mark as Send/Sync.
// After creation, Archetype is never mutated, so it can be read from multiple threads.
unsafe impl Send for Archetype {}
unsafe impl Sync for Archetype {}

impl Archetype {
    /// Creates a new archetype with the given component infos.
    pub fn new(id: usize, component_infos: Vec<ComponentInfo>) -> Self {
        let mut types: Vec<_> = component_infos.iter().map(|i| i.type_id).collect();
        types.sort();

        Self {
            id,
            types,
            table: Table::new(&component_infos),
            edges: Edges::new(),
            component_infos,
        }
    }
}

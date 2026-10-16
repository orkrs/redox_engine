use super::column::Column;
use crate::component::ComponentInfo;
use crate::entity::Entity;
use hashbrown::HashMap;
use std::any::TypeId;

/// A table storing components for a single archetype.
pub struct Table {
    pub(crate) columns: HashMap<TypeId, Column>,
    entities: Vec<Entity>,
}

// SAFETY: `Table` consists of a `HashMap<TypeId, Column>` and a `Vec<Entity>`; `Column`
// is `Send`/`Sync` (see its impls) and `Entity` is plain data. Mutation requires
// `&mut Table`, so shared references only permit reads.
unsafe impl Send for Table {}
// SAFETY: see the `Send` impl above.
unsafe impl Sync for Table {}

impl Table {
    /// Creates a new table for the given component types.
    pub fn new(component_infos: &[ComponentInfo]) -> Self {
        let mut columns = HashMap::with_capacity(component_infos.len());
        for info in component_infos {
            columns.insert(info.type_id, Column::new(*info));
        }
        Self {
            columns,
            entities: Vec::new(),
        }
    }

    /// Removes the element at `row` by swapping with the last row.
    ///
    /// When `drop` is set the removed components are dropped by their columns
    /// (despawn path). When clear, ownership of the component bytes has moved
    /// elsewhere (archetype migration) and no destructors run.
    ///
    /// Returns the entity that was moved into the vacated slot, if any.
    ///
    /// # Safety
    /// - `row` must be `< row_count()`.
    /// - Every column must have length `>= row_count()` (table invariant).
    /// - If `drop` is clear, the caller must have moved every component at `row`
    ///   out beforehand so nothing is double-dropped.
    pub unsafe fn swap_remove(&mut self, row: usize, drop: bool) -> Option<Entity> {
        let last_index = self.entities.len() - 1;
        self.entities.swap_remove(row);

        for col in self.columns.values_mut() {
            // SAFETY: `row < row_count()` (caller contract) and each column holds at
            // least one value per entity (table invariant), so `row` is in bounds.
            unsafe {
                col.swap_remove(row, drop);
            }
        }

        if row != last_index {
            Some(self.entities[row])
        } else {
            None
        }
    }

    /// Adds an entity to the table (creates a new row).
    ///
    /// # Safety
    /// Components for the new row must be added separately via `push_component_raw`
    /// (exactly one per column) before the table is read, so that every column keeps
    /// the same length as the entity list.
    pub unsafe fn push_entity(&mut self, entity: Entity) {
        self.entities.push(entity);
    }

    /// Adds a raw component value to the current last row.
    ///
    /// # Safety
    /// - `type_id` must correspond to a column in this table.
    /// - `ptr` must point to a valid instance of that component, be readable for its
    ///   size, and not overlap the column's storage.
    /// - Ownership moves into the table; the caller must not drop the source value.
    pub unsafe fn push_component_raw(&mut self, type_id: TypeId, ptr: *const u8, tick: u32) {
        if let Some(col) = self.columns.get_mut(&type_id) {
            // SAFETY: forwarded caller contract (`ptr` valid, type matches the column).
            unsafe {
                col.push(ptr, tick);
            }
        }
    }

    /// Returns a raw pointer to the component of type `type_id` at `row`.
    ///
    /// # Safety
    /// `row` must be `<= row_count()` (pointer arithmetic stays in the allocation);
    /// dereferencing the returned pointer additionally requires `row < row_count()`
    /// and that no conflicting reference to that element is live.
    pub unsafe fn get_component_ptr(&self, type_id: TypeId, row: usize) -> Option<*const u8> {
        self.columns
            .get(&type_id)
            // SAFETY: caller guarantees `row` is in bounds for this table's columns.
            .map(|col| unsafe { col.get(row) as *const u8 })
    }

    /// Returns the column for `type_id`, if present.
    pub fn column(&self, type_id: TypeId) -> Option<&Column> {
        self.columns.get(&type_id)
    }

    /// Returns the column for `type_id` mutably, if present.
    pub fn column_mut(&mut self, type_id: TypeId) -> Option<&mut Column> {
        self.columns.get_mut(&type_id)
    }

    /// Returns an iterator over all columns in this table.
    pub fn columns_mut(&mut self) -> impl Iterator<Item = &mut Column> {
        self.columns.values_mut()
    }

    /// Returns the slice of entities in this table.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Returns the number of rows.
    pub fn row_count(&self) -> usize {
        self.entities.len()
    }

    /// Checks if this table contains a column for the given `TypeId`.
    pub fn has_component(&self, type_id: TypeId) -> bool {
        self.columns.contains_key(&type_id)
    }
}

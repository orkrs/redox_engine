use crate::archetype::Archetype;
use crate::component::{Component, ComponentInfo};
use crate::entity::{Entity, EntityAllocator};
use crate::query::{NullFilter, Query, QueryData, QueryFilter};
use hashbrown::HashMap;
use std::any::{Any, TypeId};

/// Container of all entities, their components (grouped into archetypes) and singleton resources.
pub struct World {
    entities: EntityAllocator,
    entity_locations: HashMap<Entity, (usize, usize)>,
    pub(crate) archetypes: Vec<Archetype>,
    type_to_archetype: HashMap<Vec<TypeId>, usize>,

    // Хранилище глобальных ресурсов
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,

    /// Monotonic counter bumped on every component mutation.
    pub(crate) change_tick: u32,
    /// Snapshot of `change_tick` at the start of the current frame.
    ///
    /// Set by [`World::begin_frame`]; `Changed<T>` / `Added<T>` filters compare
    /// row stamps against this value ("changed this frame").
    pub(crate) frame_tick: u32,
}

impl World {
    /// Creates an empty world.
    pub fn new() -> Self {
        Self {
            entities: EntityAllocator::new(),
            entity_locations: HashMap::new(),
            archetypes: Vec::new(),
            type_to_archetype: HashMap::new(),
            resources: HashMap::new(),
            change_tick: 1,
            frame_tick: 0,
        }
    }

    /// Marks the start of a new frame for change detection.
    ///
    /// Call once per frame before systems run; afterwards every mutation bumps
    /// `change_tick`, so `Changed<T>` matches exactly what was written this frame.
    pub fn begin_frame(&mut self) {
        self.frame_tick = self.change_tick;
    }

    /// Current global change tick.
    pub fn change_tick(&self) -> u32 {
        self.change_tick
    }

    #[inline]
    pub(crate) fn next_tick(&mut self) -> u32 {
        self.change_tick = self.change_tick.wrapping_add(1).max(1);
        self.change_tick
    }

    /// Runs `f` with a [`Commands`](crate::Commands) recorder and applies everything it
    /// recorded once `f` returns. Handy for systems that iterate with shared access and
    /// need to spawn or despawn afterwards.
    ///
    /// ```
    /// # use redox_ecs::World;
    /// let mut world = World::new();
    /// let e = world.with_commands(|_world, cmds| cmds.spawn((1u32,)));
    /// assert_eq!(world.get_component::<u32>(e), Some(&1));
    /// ```
    pub fn with_commands<R>(&mut self, f: impl FnOnce(&World, &mut crate::Commands<'_>) -> R) -> R {
        let mut queue = crate::CommandQueue::new();
        let result = {
            let mut commands = crate::Commands::new(self, &mut queue);
            f(self, &mut commands)
        };
        queue.apply(self);
        result
    }

    /// Reserves a new entity id. It owns no components until some are added.
    pub fn spawn(&mut self) -> Entity {
        self.entities.allocate()
    }

    // --- ДОСТУП К КОМПОНЕНТАМ ---
    /// Returns the entity's component of type `T`, if it has one.
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let (arch_idx, row) = *self.entity_locations.get(&entity)?;
        let type_id = TypeId::of::<T>();
        let arch = &self.archetypes[arch_idx];
        if !arch.table.has_component(type_id) {
            return None;
        }
        // SAFETY: `has_component` confirmed a column keyed by `TypeId::of::<T>()`, which
        // stores exactly `T` values with `T`'s alignment. `row` comes from
        // `entity_locations`, which is kept in sync with the table, so it is `< len`.
        // The `&self` borrow prevents any `&mut` access for the returned lifetime.
        unsafe {
            let col =
                arch.table.columns.get(&type_id).expect(
                    "archetype of a located entity has a column for each of its components",
                );
            Some(&*(col.get(row) as *const T))
        }
    }

    /// Returns the entity's component of type `T` mutably and stamps it as changed.
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let (arch_idx, row) = *self.entity_locations.get(&entity)?;
        let type_id = TypeId::of::<T>();
        let tick = self.next_tick();
        let arch = &mut self.archetypes[arch_idx];
        if !arch.table.has_component(type_id) {
            return None;
        }
        // SAFETY: as in `get_component` (column type is `T`, `row` in bounds); the
        // `&mut self` borrow guarantees the returned `&mut T` is the only live
        // reference to that element.
        unsafe {
            let col =
                arch.table.columns.get_mut(&type_id).expect(
                    "archetype of a located entity has a column for each of its components",
                );
            col.set_tick(row, tick);
            Some(&mut *(col.get(row) as *mut T))
        }
    }

    // --- РЕСУРСЫ ---
    /// Inserts a resource, replacing any previous value of the same type.
    pub fn insert_resource<R: 'static + Send + Sync>(&mut self, resource: R) {
        self.resources.insert(TypeId::of::<R>(), Box::new(resource));
    }

    /// Returns the resource of type `R`, if present.
    pub fn get_resource<R: 'static + Send + Sync>(&self) -> Option<&R> {
        self.resources
            .get(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast_ref::<R>())
    }

    /// Returns the resource of type `R` mutably, if present.
    pub fn get_resource_mut<R: 'static + Send + Sync>(&mut self) -> Option<&mut R> {
        self.resources
            .get_mut(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast_mut::<R>())
    }

    /// Изымает ресурс из мира, передавая владение наружу (полезно для систем)
    pub fn remove_resource<R: 'static + Send + Sync>(&mut self) -> Option<R> {
        self.resources
            .remove(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast::<R>().ok())
            .map(|r| *r)
    }

    // --- ДОБАВЛЕНИЕ/УДАЛЕНИЕ ---

    /// Spawns an entity with all components of `bundle` in one archetype step.
    ///
    /// Avoids the per-`add_component` archetype migration chain.
    pub fn spawn_bundle<B: Bundle>(&mut self, bundle: B) -> Entity {
        let entity = self.entities.allocate();
        self.spawn_bundle_at(entity, bundle);
        entity
    }

    /// The world's entity-id allocator (lock-free; ids can be reserved through `&World`).
    pub fn entity_allocator(&self) -> &EntityAllocator {
        &self.entities
    }

    /// Spawns `bundle` onto an id previously reserved from [`Self::entity_allocator`].
    pub(crate) fn spawn_bundle_at<B: Bundle>(&mut self, entity: Entity, bundle: B) {
        let tick = self.next_tick();

        let infos = B::component_infos();
        // `push_into` pushes one value per bundle element; a duplicate component type
        // would push two values into one column and desynchronise it from the entity
        // list (breaking the table invariant that the unsafe code relies on).
        assert!(
            infos
                .iter()
                .enumerate()
                .all(|(i, a)| infos[..i].iter().all(|b| b.type_id != a.type_id)),
            "bundle contains duplicate component types"
        );
        let target = self.find_or_create_archetype(infos);

        // SAFETY: `target` was created from `B::component_infos()`, so its table has a
        // column for every bundle component; `push_entity` is immediately followed by
        // `push_into`, which pushes exactly one value per bundle column.
        unsafe {
            let arch = &mut self.archetypes[target];
            arch.table.push_entity(entity);
            bundle.push_into(arch, tick);
        }

        let row = self.archetypes[target].table.row_count() - 1;
        self.entity_locations.insert(entity, (target, row));
    }

    /// Adds a component to the entity, or overwrites the existing one (dropping the old value).
    pub fn add_component<T: Component>(&mut self, entity: Entity, component: T) {
        let component_info = ComponentInfo::of::<T>();
        let location = self.entity_locations.get(&entity).copied();

        let mut new_infos = if let Some((arch_idx, _)) = location {
            self.archetypes[arch_idx].component_infos.clone()
        } else {
            Vec::new()
        };

        if new_infos
            .iter()
            .any(|i| i.type_id == component_info.type_id)
        {
            // Overwrite in place; drop the previous value first. The new value's bytes
            // are copied into the column, so the caller-side value is wrapped in
            // `ManuallyDrop` up front: if the old value's destructor panics, unwinding
            // must not drop the new value a second time.
            if let Some((arch_idx, row)) = location {
                let tick = self.next_tick();
                let mut component = std::mem::ManuallyDrop::new(component);
                // SAFETY: the column for `T` exists (type is in the archetype) and
                // `row` is the entity's live row. `ptr` points to the local
                // `component`, a valid `T` that cannot overlap column storage; its
                // ownership moves into the column and it is never dropped here.
                unsafe {
                    let table = &mut self.archetypes[arch_idx].table;
                    if let Some(col) = table.column_mut(component_info.type_id) {
                        let ptr = &*component as *const T as *const u8;
                        col.replace(row, ptr, tick);
                    } else {
                        // No such column (cannot happen: the type is in the archetype):
                        // the value was not moved, so drop it normally.
                        std::mem::ManuallyDrop::drop(&mut component);
                    }
                }
            }
            return;
        }

        new_infos.push(component_info);
        let target_arch_idx = self.find_or_create_archetype(new_infos);
        let tick = self.next_tick();

        // SAFETY:
        // - `old_arch_idx != target_arch_idx` (the target has one more component type),
        //   and no archetype is pushed after `find_or_create_archetype`, so the two
        //   `&mut` derived from `archetypes_ptr` are to distinct, valid elements.
        // - `old_row` is the entity's live row; every old column exists in the new
        //   archetype (superset), so each `get_component_ptr(..).unwrap()` succeeds
        //   and each copied value's ownership moves to the new table. The old row is
        //   then removed with `drop = false`, so nothing is dropped twice.
        // - `component` is a valid `T` pushed into the freshly created `T` column and
        //   then forgotten, transferring ownership. Source and destination are
        //   separate allocations, so copies never overlap.
        unsafe {
            if let Some((old_arch_idx, old_row)) = location {
                let archetypes_ptr = self.archetypes.as_mut_ptr();
                let old_arch = &mut *archetypes_ptr.add(old_arch_idx);
                let new_arch = &mut *archetypes_ptr.add(target_arch_idx);

                new_arch.table.push_entity(entity);

                for info in &old_arch.component_infos {
                    let src_ptr = old_arch
                        .table
                        .get_component_ptr(info.type_id, old_row)
                        .expect("the old archetype holds every component being migrated");
                    new_arch
                        .table
                        .push_component_raw(info.type_id, src_ptr, tick);
                }

                if let Some(moved_entity) = old_arch.table.swap_remove(old_row, false) {
                    self.entity_locations
                        .insert(moved_entity, (old_arch_idx, old_row));
                }
            } else {
                self.archetypes[target_arch_idx].table.push_entity(entity);
            }

            let new_arch = &mut self.archetypes[target_arch_idx];
            let ptr = &component as *const T as *const u8;
            new_arch
                .table
                .push_component_raw(component_info.type_id, ptr, tick);
            std::mem::forget(component);
        }

        let new_row = self.archetypes[target_arch_idx].table.row_count() - 1;
        self.entity_locations
            .insert(entity, (target_arch_idx, new_row));
    }

    /// Removes (and drops) the entity's component of type `T`. Returns `false` when it had none.
    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> bool {
        // Логика удаления
        let component_info = ComponentInfo::of::<T>();
        let location = match self.entity_locations.get(&entity).copied() {
            Some(loc) => loc,
            None => return false,
        };
        let (arch_idx, row) = location;

        if !self.archetypes[arch_idx]
            .table
            .has_component(component_info.type_id)
        {
            return false;
        }

        let old_infos = &self.archetypes[arch_idx].component_infos;
        let new_infos: Vec<_> = old_infos
            .iter()
            .filter(|info| info.type_id != component_info.type_id)
            .cloned()
            .collect();

        if new_infos.is_empty() {
            self.despawn(entity);
            return true;
        }

        let target_arch_idx = self.find_or_create_archetype(new_infos);
        let tick = self.next_tick();

        // SAFETY: `arch_idx != target_arch_idx` (target lacks `T`) and both are valid
        // indices with no reallocation of `archetypes` in between, so the two `&mut`
        // are to distinct elements. `row` is the entity's live row; the new
        // archetype's columns are a subset of the old one's, so every moved component
        // exists at `row`. Moved values transfer ownership to the new table and the old
        // row is removed with `drop = false` (no double drop).
        // The removed `T` is read out first and dropped after the tables are
        // consistent again, so a panicking destructor cannot corrupt storage.
        let removed = unsafe {
            let archetypes_ptr = self.archetypes.as_mut_ptr();
            let old_arch = &mut *archetypes_ptr.add(arch_idx);
            let new_arch = &mut *archetypes_ptr.add(target_arch_idx);

            new_arch.table.push_entity(entity);

            for info in &new_arch.component_infos {
                if let Some(src_ptr) = old_arch.table.get_component_ptr(info.type_id, row) {
                    new_arch
                        .table
                        .push_component_raw(info.type_id, src_ptr, tick);
                }
            }

            let removed = old_arch
                .table
                .get_component_ptr(component_info.type_id, row)
                .map(|p| std::ptr::read(p as *const T));
            if let Some(moved_entity) = old_arch.table.swap_remove(row, false) {
                self.entity_locations.insert(moved_entity, (arch_idx, row));
            }
            removed
        };

        let new_row = self.archetypes[target_arch_idx].table.row_count() - 1;
        self.entity_locations
            .insert(entity, (target_arch_idx, new_row));
        drop(removed);
        true
    }

    /// Despawns the entity, dropping all its components. Stale handles are ignored.
    pub fn despawn(&mut self, entity: Entity) {
        if let Some((arch_idx, row)) = self.entity_locations.remove(&entity) {
            // SAFETY: `(arch_idx, row)` was just removed from `entity_locations`, which
            // always names a live row of that archetype, so `row < row_count()`.
            // `drop = true` drops each component once; the row is then gone.
            unsafe {
                let arch = &mut self.archetypes[arch_idx];
                if let Some(moved_entity) = arch.table.swap_remove(row, true) {
                    self.entity_locations.insert(moved_entity, (arch_idx, row));
                }
            }
            self.entities.deallocate(entity);
        }
    }

    fn find_or_create_archetype(&mut self, infos: Vec<ComponentInfo>) -> usize {
        let mut types: Vec<_> = infos.iter().map(|i| i.type_id).collect();
        types.sort();

        if let Some(&id) = self.type_to_archetype.get(&types) {
            id
        } else {
            let id = self.archetypes.len();
            self.archetypes.push(Archetype::new(id, infos));
            self.type_to_archetype.insert(types, id);
            id
        }
    }

    /// Iterates over every entity that currently has components.
    pub fn all_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.archetypes
            .iter()
            .flat_map(|arch| arch.table.entities().iter().copied())
    }

    /// Iterates over all archetypes, yielding for each:
    /// - the archetype index
    /// - the sorted slice of [`TypeId`]s that define the archetype
    /// - the slice of [`Entity`]s stored in that archetype
    ///
    /// Useful for building debug / inspector UIs.
    pub fn archetype_iter(&self) -> impl Iterator<Item = (usize, &[TypeId], &[Entity])> + '_ {
        self.archetypes
            .iter()
            .enumerate()
            .map(|(i, arch)| (i, arch.component_types(), arch.entities()))
    }

    /// Number of active archetypes.
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    // --- QUERIES ---

    /// Creates a query for the given [`QueryData`] type.
    ///
    /// # Example
    /// ```rust,ignore
    /// // Read-only fetches iterate with a shared world borrow
    /// for pos in world.query::<&Position>().iter(&world) { ... }
    ///
    /// // Mutable fetches require exclusive access
    /// for (pos, vel) in world.query::<(&Position, &mut Velocity)>().iter_mut(&mut world) { ... }
    /// ```
    #[inline]
    pub fn query<Q: QueryData>(&self) -> Query<Q, NullFilter> {
        Query::new()
    }

    /// Creates a filtered query for the given [`QueryData`] and [`QueryFilter`] types.
    ///
    /// # Example
    /// ```rust,ignore
    /// for (pos, vel) in world.query_filtered::<(&Position, &Velocity), With<Player>>().iter(&world) { ... }
    /// ```
    #[inline]
    pub fn query_filtered<Q: QueryData, F: QueryFilter>(&self) -> Query<Q, F> {
        Query::new()
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Bundles – spawn an entity with many components in one archetype transition
// ---------------------------------------------------------------------------

/// A fixed-size collection of components that can be spawned together.
///
/// Implemented via macro for tuples up to 8 elements.
pub trait Bundle {
    /// Metadata for every component in the bundle (order-stable).
    fn component_infos() -> Vec<ComponentInfo>;

    /// Pushes every component into the archetype's current last row.
    ///
    /// # Safety
    /// The archetype's table must have had an entity pushed (with no components yet
    /// for that row) and must contain a column for every component of this bundle,
    /// with each bundle component type appearing at most once. Ownership of all
    /// components moves into the archetype.
    unsafe fn push_into(self, archetype: &mut Archetype, tick: u32);
}

impl Bundle for () {
    fn component_infos() -> Vec<ComponentInfo> {
        Vec::new()
    }

    /// # Safety
    /// No requirements: an empty bundle touches nothing.
    unsafe fn push_into(self, _archetype: &mut Archetype, _tick: u32) {}
}

macro_rules! impl_bundle {
    ($( $name:ident : $idx:tt ),+) => {
        #[allow(non_snake_case)]
        impl<$($name: Component),+> Bundle for ($($name,)+) {
            fn component_infos() -> Vec<ComponentInfo> {
                vec![ $( ComponentInfo::of::<$name>(), )+ ]
            }

            /// # Safety
            /// See [`Bundle::push_into`].
            unsafe fn push_into(self, archetype: &mut Archetype, tick: u32) {
                $(
                    let value = self.$idx;
                    let ptr = &value as *const $name as *const u8;
                    // SAFETY: archetype's table has a column for every bundle
                    // component (guaranteed by `component_infos`) and a freshly
                    // pushed entity row; `ptr` points to the local `value`, a valid
                    // component whose ownership moves into the column and which is
                    // forgotten immediately below.
                    unsafe {
                        archetype.table.push_component_raw(TypeId::of::<$name>(), ptr, tick);
                    }
                    std::mem::forget(value);
                )+
            }
        }
    };
}

impl_bundle!(A:0);
impl_bundle!(A:0, B:1);
impl_bundle!(A:0, B:1, C:2);
impl_bundle!(A:0, B:1, C:2, D:3);
impl_bundle!(A:0, B:1, C:2, D:3, E:4);
impl_bundle!(A:0, B:1, C:2, D:3, E:4, F:5);
impl_bundle!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
impl_bundle!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct CompA(i32);

    #[derive(Debug, Clone, PartialEq)]
    struct CompB(f32);

    #[test]
    fn test_archetype_migration_preserves_components() {
        let mut world = World::new();
        let entity = world.spawn();

        world.add_component(entity, CompA(42));
        world.add_component(entity, CompB(2.5));

        assert_eq!(world.get_component::<CompA>(entity).unwrap().0, 42);
        assert_eq!(world.get_component::<CompB>(entity).unwrap().0, 2.5);
    }
}

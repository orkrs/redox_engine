use crate::archetype::Archetype;
use crate::component::{Component, ComponentInfo};
use crate::entity::{Entity, EntityAllocator};
use hashbrown::HashMap;
use std::any::{Any, TypeId};

pub struct World {
    entities: EntityAllocator,
    entity_locations: HashMap<Entity, (usize, usize)>,
    pub(crate) archetypes: Vec<Archetype>,
    type_to_archetype: HashMap<Vec<TypeId>, usize>,

    // Хранилище глобальных ресурсов
    resources: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            entities: EntityAllocator::new(),
            entity_locations: HashMap::new(),
            archetypes: Vec::new(),
            type_to_archetype: HashMap::new(),
            resources: HashMap::new(),
        }
    }

    pub fn spawn(&mut self) -> Entity {
        self.entities.allocate()
    }

    // --- ДОСТУП К КОМПОНЕНТАМ ---
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let (arch_idx, row) = *self.entity_locations.get(&entity)?;
        let type_id = TypeId::of::<T>();
        let arch = &self.archetypes[arch_idx];
        if !arch.table.has_component(type_id) {
            return None;
        }
        unsafe {
            let col = arch.table.columns.get(&type_id).unwrap();
            Some(&*(col.get(row) as *const T))
        }
    }

    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let (arch_idx, row) = *self.entity_locations.get(&entity)?;
        let type_id = TypeId::of::<T>();
        let arch = &mut self.archetypes[arch_idx];
        if !arch.table.has_component(type_id) {
            return None;
        }
        unsafe {
            let col = arch.table.columns.get_mut(&type_id).unwrap();
            Some(&mut *(col.get(row) as *mut T))
        }
    }

    // --- РЕСУРСЫ ---
    pub fn insert_resource<R: 'static + Send + Sync>(&mut self, resource: R) {
        self.resources.insert(TypeId::of::<R>(), Box::new(resource));
    }

    pub fn get_resource<R: 'static + Send + Sync>(&self) -> Option<&R> {
        self.resources
            .get(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast_ref::<R>())
    }

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
            if let Some((arch_idx, row)) = location {
                let table = &mut self.archetypes[arch_idx].table;
                unsafe {
                    let col = table.columns.get_mut(&component_info.type_id).unwrap();
                    *(col.get(row) as *mut T) = component;
                }
            }
            return;
        }

        new_infos.push(component_info.clone());
        let target_arch_idx = self.find_or_create_archetype(new_infos);

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
                        .unwrap();
                    new_arch.table.push_component_raw(info.type_id, src_ptr);
                }

                if let Some(moved_entity) = old_arch.table.swap_remove(old_row) {
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
                .push_component_raw(component_info.type_id, ptr);
            std::mem::forget(component);
        }

        let new_row = self.archetypes[target_arch_idx].table.row_count() - 1;
        self.entity_locations
            .insert(entity, (target_arch_idx, new_row));
    }

    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> bool {
        // Логика удаления (остается твоей)
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

        unsafe {
            let archetypes_ptr = self.archetypes.as_mut_ptr();
            let old_arch = &mut *archetypes_ptr.add(arch_idx);
            let new_arch = &mut *archetypes_ptr.add(target_arch_idx);

            new_arch.table.push_entity(entity);

            for info in &new_arch.component_infos {
                if let Some(src_ptr) = old_arch.table.get_component_ptr(info.type_id, row) {
                    new_arch.table.push_component_raw(info.type_id, src_ptr);
                }
            }

            if let Some(moved_entity) = old_arch.table.swap_remove(row) {
                self.entity_locations.insert(moved_entity, (arch_idx, row));
            }
        }

        let new_row = self.archetypes[target_arch_idx].table.row_count() - 1;
        self.entity_locations
            .insert(entity, (target_arch_idx, new_row));
        true
    }

    pub fn despawn(&mut self, entity: Entity) {
        if let Some((arch_idx, row)) = self.entity_locations.remove(&entity) {
            unsafe {
                let arch = &mut self.archetypes[arch_idx];
                if let Some(moved_entity) = arch.table.swap_remove(row) {
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
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

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
        world.add_component(entity, CompB(3.14));

        assert_eq!(world.get_component::<CompA>(entity).unwrap().0, 42);
        assert_eq!(world.get_component::<CompB>(entity).unwrap().0, 3.14);
    }
}

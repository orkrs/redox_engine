use std::collections::{HashMap, HashSet, VecDeque};

use smallvec::SmallVec;

use crate::entity::Entity;
use crate::query::filter::Changed;
use crate::world::World;
use redox_math::{GlobalTransform, Mat4, Transform};

/// Component added to an entity to point to its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parent(pub Entity);

/// Component added to an entity to track its children.
#[derive(Debug, Clone)]
pub struct Children {
    pub list: SmallVec<[Entity; 8]>,
}

impl Children {
    /// Creates an empty children list.
    pub fn new() -> Self {
        Self {
            list: SmallVec::new(),
        }
    }
}

impl Default for Children {
    fn default() -> Self {
        Self::new()
    }
}

/// Attaches `child` under `parent`, updating both [`Parent`] and [`Children`].
pub fn set_parent(world: &mut World, child: Entity, parent: Entity) {
    if child == parent {
        return;
    }
    if let Some(Parent(old)) = world.get_component::<Parent>(child).copied() {
        if old == parent {
            return;
        }
        remove_from_children_list(world, old, child);
    }
    world.add_component(child, Parent(parent));
    let mut kids = world
        .get_component::<Children>(parent)
        .cloned()
        .unwrap_or_default();
    if !kids.list.contains(&child) {
        kids.list.push(child);
    }
    world.add_component(parent, kids);
}

fn remove_from_children_list(world: &mut World, parent: Entity, child: Entity) {
    if let Some(mut kids) = world.get_component::<Children>(parent).cloned() {
        kids.list.retain(|e| *e != child);
        world.add_component(parent, kids);
    }
}

use std::cell::RefCell;

struct HierarchyScratch {
    with_transform: Vec<Entity>,
    dirty: HashSet<Entity>,
    flood: VecDeque<Entity>,
    dirties: Vec<Entity>,
    cache: HashMap<Entity, Mat4>,
    visiting: HashSet<Entity>,
}

thread_local! {
    static HIERARCHY_SCRATCH: RefCell<HierarchyScratch> = RefCell::new(HierarchyScratch {
        with_transform: Vec::with_capacity(256),
        dirty: HashSet::with_capacity(128),
        flood: VecDeque::with_capacity(128),
        dirties: Vec::with_capacity(128),
        cache: HashMap::with_capacity(128),
        visiting: HashSet::with_capacity(64),
    });
}

/// Writes [`GlobalTransform`] = parent_global × local [`Transform`].
///
/// Walks dirty subtrees (`Changed<Transform>` / `Changed<Parent>` / missing global)
/// using preallocated scratch buffers without heap allocations in the frame loop.
pub fn transform_hierarchy_system(world: &mut World) {
    HIERARCHY_SCRATCH.with(|scratch| {
        let mut s = scratch.borrow_mut();
        let HierarchyScratch {
            with_transform,
            dirty,
            flood,
            dirties,
            cache,
            visiting,
        } = &mut *s;

        with_transform.clear();
        dirty.clear();
        flood.clear();
        dirties.clear();
        cache.clear();
        visiting.clear();

        for (e, _) in world.query::<(Entity, &Transform)>().iter(world) {
            with_transform.push(e);
            if world.get_component::<GlobalTransform>(e).is_none() {
                dirty.insert(e);
            }
        }

        for e in world
            .query_filtered::<Entity, Changed<Transform>>()
            .iter(world)
        {
            dirty.insert(e);
        }
        for e in world
            .query_filtered::<Entity, Changed<Parent>>()
            .iter(world)
        {
            dirty.insert(e);
        }

        if dirty.is_empty() {
            return;
        }

        flood.extend(dirty.iter().copied());
        while let Some(e) = flood.pop_front() {
            if let Some(children) = world.get_component::<Children>(e) {
                for &child in &children.list {
                    if dirty.insert(child) {
                        flood.push_back(child);
                    }
                }
            }
        }

        dirties.extend(dirty.iter().copied());
        for &e in dirties.iter() {
            let mat = compose_global(world, e, cache, visiting);
            match world.get_component_mut::<GlobalTransform>(e) {
                Some(g) => g.matrix = mat,
                None => world.add_component(e, GlobalTransform::from_matrix(mat)),
            }
        }
    });
}

fn compose_global(
    world: &World,
    entity: Entity,
    cache: &mut HashMap<Entity, Mat4>,
    visiting: &mut HashSet<Entity>,
) -> Mat4 {
    if let Some(m) = cache.get(&entity) {
        return *m;
    }
    if !visiting.insert(entity) {
        return world
            .get_component::<Transform>(entity)
            .map(|t| t.matrix())
            .unwrap_or(Mat4::IDENTITY);
    }
    let local = world
        .get_component::<Transform>(entity)
        .map(|t| t.matrix())
        .unwrap_or(Mat4::IDENTITY);
    let world_mat = if let Some(Parent(parent)) = world.get_component::<Parent>(entity).copied() {
        compose_global(world, parent, cache, visiting) * local
    } else {
        local
    };
    visiting.remove(&entity);
    cache.insert(entity, world_mat);
    world_mat
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;

    #[test]
    fn child_follows_parent() {
        let mut world = World::new();
        world.begin_frame();
        let parent = world.spawn();
        let child = world.spawn();
        world.add_component(
            parent,
            Transform::from_translation(Vec3::new(10.0, 0.0, 0.0)),
        );
        world.add_component(child, Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)));
        set_parent(&mut world, child, parent);

        transform_hierarchy_system(&mut world);
        let g = world.get_component::<GlobalTransform>(child).unwrap();
        let t = g.translation();
        assert!((t.x - 11.0).abs() < 1e-4, "got {t:?}");

        world.begin_frame();
        world
            .get_component_mut::<Transform>(parent)
            .unwrap()
            .translation
            .x = 20.0;
        transform_hierarchy_system(&mut world);
        let t = world
            .get_component::<GlobalTransform>(child)
            .unwrap()
            .translation();
        assert!((t.x - 21.0).abs() < 1e-4, "got {t:?}");
    }
}

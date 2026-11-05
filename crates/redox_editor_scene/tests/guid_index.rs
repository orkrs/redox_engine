//! Headless tests of `GuidIndex` and `assign_missing_guids`.

use redox_ecs::World;
use redox_editor_scene::guid_index::{GuidIndex, assign_missing_guids};
use redox_math::Transform;
use redox_scene::EntityGuid;

#[test]
fn index_finds_an_entity_after_spawn_and_forgets_it_after_despawn() {
    let mut world = World::new();
    let entity = world.spawn();
    let guid = EntityGuid::new_random();
    world.add_component(entity, guid);

    let mut index = GuidIndex::new();
    index.rebuild(&world);
    assert_eq!(index.get(guid), Some(entity));
    assert_eq!(index.guid_of(entity), Some(guid));
    assert_eq!(index.len(), 1);

    world.despawn(entity);
    index.rebuild(&world);
    assert_eq!(index.get(guid), None);
    assert!(index.is_empty());
}

#[test]
fn explicit_insert_and_remove_keep_both_directions_in_step() {
    let mut world = World::new();
    let entity = world.spawn();
    world.add_component(entity, Transform::default());
    let guid = EntityGuid::new_random();

    let mut index = GuidIndex::new();
    index.insert(guid, entity);
    assert_eq!(index.get(guid), Some(entity));
    assert_eq!(index.guid_of(entity), Some(guid));

    index.remove_entity(entity);
    assert_eq!(index.get(guid), None);
    assert_eq!(index.guid_of(entity), None);
}

#[test]
fn inserting_a_guid_for_another_entity_moves_it() {
    let mut world = World::new();
    let first = world.spawn();
    let second = world.spawn();
    let guid = EntityGuid::new_random();

    let mut index = GuidIndex::new();
    index.insert(guid, first);
    index.insert(guid, second);

    assert_eq!(index.get(guid), Some(second));
    assert_eq!(index.guid_of(first), None);
    assert_eq!(index.len(), 1);
}

#[test]
fn assign_missing_guids_gives_ids_only_to_entities_without_one() {
    let mut world = World::new();
    let with_id = world.spawn();
    let existing = EntityGuid::new_random();
    world.add_component(with_id, existing);
    let without_id = world.spawn();
    world.add_component(without_id, Transform::default());

    assert_eq!(assign_missing_guids(&mut world), 1);

    let assigned = world
        .get_component::<EntityGuid>(without_id)
        .copied()
        .expect("the entity received a guid");
    assert_ne!(assigned, existing);
    assert_eq!(
        world.get_component::<EntityGuid>(with_id).copied(),
        Some(existing)
    );
    assert_eq!(assign_missing_guids(&mut world), 0);
}

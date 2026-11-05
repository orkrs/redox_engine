//! Scene component registration: every scene type has a schema and survives a registry snapshot.

use redox_ecs::World;
use redox_editor_scene::register_scene_components;
use redox_math::{Transform, Vec3};
use redox_reflect::TypeRegistry;
use redox_render::{Camera, DirectionalLight, MeshHandle, PointLight, SpotLight};
use redox_scene::{Name, Tag};

#[test]
fn every_scene_component_type_is_registered() {
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);

    for name in [
        "Transform",
        "Name",
        "Tag",
        "DirectionalLight",
        "PointLight",
        "SpotLight",
        "Camera",
        "MeshHandle",
        "MaterialHandle",
    ] {
        assert!(registry.schema(name).is_some(), "{name} is not registered");
    }
}

#[test]
fn a_scene_entity_with_every_component_roundtrips_through_the_registry() {
    let mut registry = TypeRegistry::new();
    register_scene_components(&mut registry);

    let mut world = World::new();
    let entity = world.spawn();
    world.add_component(entity, Name("sun".into()));
    world.add_component(entity, Tag("light".into()));
    world.add_component(entity, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)));
    world.add_component(
        entity,
        DirectionalLight::new(Vec3::new(0.0, 1.0, 0.0), Vec3::ONE, 1.0),
    );
    world.add_component(entity, PointLight::new(Vec3::ONE, Vec3::ONE, 2.0, 4.0));
    world.add_component(entity, SpotLight::default());
    world.add_component(entity, Camera::default());
    world.add_component(entity, MeshHandle::default());

    let before = registry.snapshot_world(&world);
    let text = registry.to_ron(&world).expect("to_ron");
    let mut restored = World::new();
    registry.from_ron(&mut restored, &text).expect("from_ron");

    assert_eq!(registry.snapshot_world(&restored), before);
}

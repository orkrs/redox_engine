//! Registration of the scene component types for reflection, saving and the inspector.

use redox_ecs::{Entity, World};
use redox_gameplay::AttributeSet;
use redox_physics::Velocity;
use redox_reflect::{ReflectValue, TypeRegistry, register_engine_defaults};
use redox_render::{Camera, DirectionalLight, MaterialHandle, MeshHandle, PointLight, SpotLight};
use redox_scene::{AssetRefs, Name, Tag};

fn read_handle_value(
    world: &World,
    entity: Entity,
    type_name: &'static str,
) -> Option<ReflectValue> {
    let has_handle = match type_name {
        "MeshHandle" => world.get_component::<MeshHandle>(entity).is_some(),
        "MaterialHandle" => world.get_component::<MaterialHandle>(entity).is_some(),
        _ => return None,
    };
    if !has_handle {
        return None;
    }
    if let Some(refs) = world.get_component::<AssetRefs>(entity)
        && let Some(saved) = refs.get(type_name)
    {
        return Some(ReflectValue::asset_ref(
            type_name,
            saved.guid.clone(),
            saved.path.clone(),
        ));
    }
    Some(ReflectValue::handle(type_name, None))
}

/// Registers the engine defaults (`Transform`) and every scene component type.
///
/// Components that hold only runtime handles (rigid bodies, colliders, state trees, editor
/// mesh proxies) are not registered: they have no saved form yet, see `KNOWN_LIMITS.md`.
pub fn register_scene_components(registry: &mut TypeRegistry) {
    register_engine_defaults(registry);
    registry.register::<Name>();
    registry.register::<Tag>();
    registry.register::<DirectionalLight>();
    registry.register::<PointLight>();
    registry.register::<SpotLight>();
    registry.register::<Camera>();
    registry.register::<MeshHandle>();
    registry.register::<MaterialHandle>();
    registry.register::<AssetRefs>();
    registry.register::<Velocity>();
    registry.register::<AttributeSet>();
    registry.override_read("MeshHandle", |world, entity| {
        read_handle_value(world, entity, "MeshHandle")
    });
    registry.override_read("MaterialHandle", |world, entity| {
        read_handle_value(world, entity, "MaterialHandle")
    });
}

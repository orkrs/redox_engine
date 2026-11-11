//! Registration of the scene component types for reflection, saving and the inspector.

use redox_gameplay::AttributeSet;
use redox_physics::Velocity;
use redox_reflect::{TypeRegistry, register_engine_defaults};
use redox_render::{Camera, DirectionalLight, MaterialHandle, MeshHandle, PointLight, SpotLight};
use redox_scene::{AssetRefs, Name, Tag};

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
}

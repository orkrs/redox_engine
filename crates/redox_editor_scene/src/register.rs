//! Registration of the scene component types for reflection, saving and the inspector.

use redox_reflect::{TypeRegistry, register_engine_defaults};
use redox_render::{Camera, DirectionalLight, MaterialHandle, MeshHandle, PointLight, SpotLight};
use redox_scene::{Name, Tag};

/// Registers the engine defaults (`Transform`) and every scene component type.
///
/// Physics, audio and gameplay types are registered by later slices.
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
}

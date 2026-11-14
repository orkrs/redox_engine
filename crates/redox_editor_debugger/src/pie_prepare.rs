//! Helpers to apply PIE settings to a world before the session starts.

use redox_ecs::World;
use redox_editor_scene::Level;
use redox_reflect::TypeRegistry;

use crate::pie_settings::{PieArgs, PieSettings};

/// Result of optionally loading a PIE start scene into `world` (sandbox copy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PieSceneLoad {
    /// No override; the open document world is used as-is.
    Skipped,
    /// Scene file loaded into `world`.
    Loaded,
    /// Override path missing or unreadable.
    Failed(String),
}

/// Writes [`PieArgs`] into the world and optionally loads `settings.start_scene`.
///
/// When `start_scene` is set, the current `world` is replaced by the scene contents (the
/// editor document on disk is not modified).
pub fn prepare_pie_world(
    settings: &PieSettings,
    project_root: Option<&std::path::Path>,
    registry: &TypeRegistry,
    world: &mut World,
) -> (PieArgs, PieSceneLoad) {
    let args = PieArgs {
        args: settings.args.clone(),
    };
    world.insert_resource(args.clone());
    let mut load = PieSceneLoad::Skipped;
    if let Some(root) = project_root {
        if let Some(path) = settings.start_scene_path(root) {
            if path.is_file() {
                *world = World::new();
                match Level::open(&path, registry, world) {
                    Ok((_level, _report)) => load = PieSceneLoad::Loaded,
                    Err(err) => load = PieSceneLoad::Failed(format!("{err:?}")),
                }
            } else {
                load = PieSceneLoad::Failed(format!("missing start scene {}", path.display()));
            }
        }
        let world_bin = root.join(".redox/world.bin");
        let bin = world_bin.is_file().then_some(world_bin.as_path());
        redox_core::boot_navigation(world, root, bin, "release");
        redox_core::boot_reflection_probes(world);
    }
    (args, load)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_editor_scene::register_scene_components;
    use redox_math::Transform;
    use redox_scene::Name;
    use std::fs;
    use std::path::Path;

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_scene_components(&mut registry);
        registry
    }

    #[test]
    fn override_scene_loads_into_sandbox_world() {
        let dir = std::env::temp_dir().join(format!("redox_pie_load_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let scene = dir.join("override.scene.ron");
        {
            let registry = registry();
            let mut world = World::new();
            let e = world.spawn();
            world.add_component(e, Name("PieOnly".into()));
            world.add_component(e, Transform::default());
            let level = redox_editor_scene::Level::new_scene("override", &registry, &world);
            let mut doc_level = level;
            doc_level
                .save_as(&scene, &registry, &mut world)
                .expect("save scene");
        }
        let settings = PieSettings {
            start_scene: Some("override.scene.ron".into()),
            args: vec!["--pie".into()],
        };
        let registry = registry();
        let mut world = World::new();
        let (_args, load) = prepare_pie_world(&settings, Some(&dir), &registry, &mut world);
        assert_eq!(load, PieSceneLoad::Loaded);
        let names: Vec<_> = world
            .all_entities()
            .filter_map(|e| world.get_component::<Name>(e).map(|n| n.0.clone()))
            .collect();
        assert!(names.iter().any(|n| n == "PieOnly"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn args_are_inserted_as_resource() {
        let registry = registry();
        let mut world = World::new();
        let settings = PieSettings {
            start_scene: None,
            args: vec!["--verbose".into()],
        };
        prepare_pie_world(&settings, None, &registry, &mut world);
        let args = world.get_resource::<PieArgs>().expect("PieArgs resource");
        assert!(args.has_flag("--verbose"));
    }
}

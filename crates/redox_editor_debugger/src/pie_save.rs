//! Guards persistent saves while PIE is running.

use crate::PieSession;
use redox_ecs::World;
use redox_editor_scene::Level;
use redox_reflect::TypeRegistry;

/// Why a level save was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PieSaveBlock {
    /// Save proceeded (PIE not running).
    Allowed,
    /// PIE is active; the level file and baseline must not change.
    BlockedByPie,
}

/// Returns whether [`Level::save`] / `mark_saved` may update persistent state.
pub fn save_guard(pie: &PieSession) -> PieSaveBlock {
    if pie.blocks_persistent_save() {
        PieSaveBlock::BlockedByPie
    } else {
        PieSaveBlock::Allowed
    }
}

/// Saves the level when PIE allows it; otherwise leaves disk and baseline untouched.
pub fn save_level_if_allowed(
    pie: &PieSession,
    level: &mut Level,
    registry: &TypeRegistry,
    world: &mut World,
) -> Result<PieSaveBlock, redox_editor_scene::LevelError> {
    match save_guard(pie) {
        PieSaveBlock::BlockedByPie => Ok(PieSaveBlock::BlockedByPie),
        PieSaveBlock::Allowed => {
            level.save(registry, world)?;
            Ok(PieSaveBlock::Allowed)
        }
    }
}

/// Applies `mark_saved` on the undo stack only when PIE is stopped.
pub fn mark_saved_if_allowed(
    pie: &PieSession,
    undo: &mut redox_editor_core::UndoStack,
) -> PieSaveBlock {
    match save_guard(pie) {
        PieSaveBlock::BlockedByPie => PieSaveBlock::BlockedByPie,
        PieSaveBlock::Allowed => {
            undo.mark_saved();
            PieSaveBlock::Allowed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_editor_scene::register_scene_components;
    use redox_math::Transform;
    use std::fs;

    #[test]
    fn mark_saved_during_play_does_not_change_level_file() {
        let dir = std::env::temp_dir().join(format!("redox_pie_save_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("test.scene.ron");

        let mut registry = TypeRegistry::new();
        register_scene_components(&mut registry);
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::default());
        let mut level = Level::new_scene("test", &registry, &world);
        level.save_as(&path, &registry, &mut world).expect("save");

        let before = fs::read_to_string(&path).expect("read");
        world
            .get_component_mut::<Transform>(e)
            .expect("transform")
            .translation
            .x = 42.0;
        assert!(level.is_dirty(&registry, &world));

        let mut pie = PieSession::new();
        pie.start(&world, &registry);
        assert_eq!(
            save_level_if_allowed(&pie, &mut level, &registry, &mut world).unwrap(),
            PieSaveBlock::BlockedByPie
        );

        let after = fs::read_to_string(&path).expect("read again");
        assert_eq!(
            before, after,
            "level file on disk must be unchanged during PIE"
        );
        pie.stop(&mut world, &registry);
        let _ = fs::remove_dir_all(&dir);
    }
}

//! Sidecar loads (`.nav`, reflection probe dirty flags) after a level folder is opened.

use std::path::Path;

use redox_ecs::World;

use crate::nav_load::{cooked_nav_path, insert_nav_from_path, nav_path_beside_world_bin};
use crate::reflection_probes::mark_all_reflection_probes_dirty;

/// Inserts navigation from the profile cook output or a sibling of `world_bin`.
pub fn boot_navigation(
    world: &mut World,
    level_root: &Path,
    world_bin: Option<&Path>,
    profile: &str,
) {
    if let Some(bin) = world_bin {
        let path = nav_path_beside_world_bin(bin, profile);
        insert_nav_from_path(world, &path);
    }
    let cooked = cooked_nav_path(level_root, profile);
    if world.get_resource::<redox_nav::NavWorld>().is_none() {
        insert_nav_from_path(world, &cooked);
    }
}

/// Marks every reflection probe dirty once after a level load (first captures).
pub fn boot_reflection_probes(world: &mut World) {
    mark_all_reflection_probes_dirty(world);
}

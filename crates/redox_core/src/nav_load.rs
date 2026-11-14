//! Load baked `.nav` files into an ECS [`NavWorld`] resource.

use std::fs;
use std::path::{Path, PathBuf};

use redox_ecs::World;
use redox_nav::{NavData, NavWorld};

/// Parses `.nav` bytes and builds a [`NavWorld`].
///
/// # Errors
/// Returns a message when the format is invalid.
pub fn load_nav_world(bytes: &[u8]) -> Result<NavWorld, String> {
    let data = NavData::from_bytes(bytes).map_err(|e| e.to_string())?;
    NavWorld::from_data(&data).map_err(|e| e.to_string())
}

/// When `path` exists, reads it and inserts [`NavWorld`]. Missing files are ignored.
pub fn insert_nav_from_path(world: &mut World, path: &Path) {
    if !path.is_file() {
        return;
    }
    match fs::read(path) {
        Ok(bytes) => match load_nav_world(&bytes) {
            Ok(nav) => {
                world.insert_resource(nav);
            }
            Err(message) => {
                log::warn!("navigation stays off: {}: {message}", path.display());
            }
        },
        Err(err) => {
            log::warn!("navigation stays off: cannot read {}: {err}", path.display());
        }
    }
}

/// Default cook output: `<level_root>/.redox/<profile>.nav`.
pub fn cooked_nav_path(level_root: &Path, profile: &str) -> PathBuf {
    level_root.join(".redox").join(format!("{profile}.nav"))
}

/// Resolves a nav mesh next to a binary world: `.redox/<profile>.nav`, then `<stem>.nav`.
pub fn nav_path_beside_world_bin(world_bin: &Path, profile: &str) -> PathBuf {
    let dir = world_bin.parent().unwrap_or_else(|| Path::new("."));
    let cooked = dir.join(".redox").join(format!("{profile}.nav"));
    if cooked.is_file() {
        return cooked;
    }
    let sibling = world_bin.with_extension("nav");
    if sibling.is_file() {
        return sibling;
    }
    cooked
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_math::Vec3;
    use redox_nav::{AgentClass, AgentParams, NavAreaId, NavData, NavPolygon, NavTile, NavWorld};

    fn corridor(cols: u16) -> NavData {
        let mut vertices = Vec::new();
        for r in 0..=1u16 {
            for c in 0..=cols {
                vertices.push([f32::from(c), 0.0, f32::from(r)]);
            }
        }
        let stride = cols + 1;
        let polygons = (0..cols)
            .map(|c| NavPolygon {
                indices: vec![c, stride + c, stride + c + 1, c + 1],
                area: NavAreaId::WALKABLE,
            })
            .collect();
        NavData {
            agent: AgentParams::default(),
            tile_size: 16.0,
            tiles: vec![NavTile {
                coord: (0, 0),
                vertices,
                polygons,
            }],
        }
    }

    #[test]
    fn missing_nav_file_leaves_the_world_without_nav() {
        let mut world = World::new();
        insert_nav_from_path(&mut world, Path::new("/nonexistent/level.nav"));
        assert!(world.get_resource::<NavWorld>().is_none());
    }

    #[test]
    fn corrupt_nav_bytes_are_an_error() {
        assert!(load_nav_world(b"not a nav file").is_err());
    }

    #[test]
    fn a_baked_corridor_roundtrips_into_a_path() {
        let bytes = corridor(8).to_bytes();
        let nav = load_nav_world(&bytes).expect("parse");
        let points = nav
            .find_path(
                Vec3::new(0.5, 0.0, 0.5),
                Vec3::new(7.5, 0.0, 0.5),
                AgentClass(0),
            )
            .expect("path");
        assert!(points.len() >= 2);
        let roundtrip = NavWorld::from_data(&NavData::from_bytes(&bytes).unwrap()).unwrap();
        let _ = roundtrip;
    }

    #[test]
    fn pie_or_game_boot_inserts_navworld_when_the_file_exists() {
        let dir = std::env::temp_dir().join(format!("redox_nav_boot_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("level.nav");
        fs::write(&path, corridor(4).to_bytes()).unwrap();
        let mut world = World::new();
        insert_nav_from_path(&mut world, &path);
        assert!(world.get_resource::<NavWorld>().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn boot_without_a_file_does_not_panic() {
        let mut world = World::new();
        insert_nav_from_path(&mut world, Path::new("nope.nav"));
        assert!(world.get_resource::<NavWorld>().is_none());
    }
}
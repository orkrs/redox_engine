//! `bake-hlod`: the command shell for per-cell HLOD proxies.
//!
//! Opens a binary world and reports its cells. Building the proxies needs a
//! link from each actor to its geometry; `ActorRecord` has none yet (only id,
//! kind, position, yaw and tags), so the bake stops with an explanation
//! instead of guessing a format.

use std::path::Path;

use redox_scene::world_bin::BinWorld;

/// Default triangle budget of one cell proxy.
pub const DEFAULT_HLOD_TRIANGLES: usize = 2000;

/// Parses the options after `bake-hlod <world> <out>`: nothing, or `--triangles N`.
pub fn parse_triangles(flags: &[&str]) -> Result<usize, String> {
    match flags {
        [] => Ok(DEFAULT_HLOD_TRIANGLES),
        ["--triangles", value] => value
            .parse::<usize>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or_else(|| format!("bad --triangles value: {value}")),
        other => Err(format!("unknown option: {}", other.join(" "))),
    }
}

/// Bakes one `.hlod` proxy per cell of the binary world `world` into `out`.
///
/// Currently always fails after opening the world, because cells carry no
/// geometry reference to merge.
pub fn bake_hlod(world: &Path, out: &Path, triangles: usize) -> Result<(), String> {
    let bin = BinWorld::open(world).map_err(|e| format!("{}: {e}", world.display()))?;
    let cells = bin.cells().count();
    Err(format!(
        "{cells} cells in {}: ActorRecord has no geometry reference, so no {triangles}-triangle \
         proxy can be built into {} yet (the actor-to-mesh binding is an open question)",
        world.display(),
        out.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_triangle_count_is_used_without_flags() {
        assert_eq!(parse_triangles(&[]).unwrap(), 2000);
    }

    #[test]
    fn a_triangle_count_flag_is_parsed() {
        assert_eq!(parse_triangles(&["--triangles", "500"]).unwrap(), 500);
    }

    #[test]
    fn bad_flags_are_errors() {
        assert!(parse_triangles(&["--triangles"]).is_err());
        assert!(parse_triangles(&["--triangles", "many"]).is_err());
        assert!(parse_triangles(&["--wat", "1"]).is_err());
    }
}

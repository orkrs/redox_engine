//! The game's content, generated at startup and packed into a `.pak`: the level text and two
//! KTX2 textures with full mip chains. Everything is then read back through the pack, the way a
//! shipped game would.

use redox_asset::{Ktx2Texture, PakError, PakWriter, TextureFormat};

pub const LEVEL: &str = "level.txt";
pub const GROUND: &str = "ground.ktx2";
pub const CRATE: &str = "crate.ktx2";

const LEVEL_TEXT: &str = "\
# Courier level 1
arena 14
spawn 0 -9
crate 0 -3 1.0
crate -5 2 1.2
crate 6 4 1.0
crate 2 8 0.8
crate -8 -6 0.9
coin 0 -1
coin -5 -1
coin -8 5
coin -3 9
coin 4 1
coin 9 8
coin 10 -4
coin -10 -10
";

fn hash(x: u32, y: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

/// A mossy checkerboard: big tiles, grid lines and per-pixel speckle so that every mip differs.
fn pixel(x: u32, y: u32, size: u32, base: [f32; 3]) -> [u8; 4] {
    let tile = (size / 16).max(1);
    let checker = ((x / tile) + (y / tile)) % 2 == 0;
    let line = x % tile < 2.max(tile / 32) || y % tile < 2.max(tile / 32);
    let speckle = (hash(x, y) & 0xff) as f32 / 255.0;
    let shade =
        if checker { 1.0 } else { 0.72 } * (0.85 + 0.3 * speckle) * if line { 0.55 } else { 1.0 };
    let to_byte = |c: f32| (c * shade * 255.0).clamp(0.0, 255.0) as u8;
    [to_byte(base[0]), to_byte(base[1]), to_byte(base[2]), 255]
}

fn texture(size: u32, base: [f32; 3]) -> Ktx2Texture {
    let mut levels = Vec::new();
    let mut current: Vec<u8> = (0..size * size)
        .flat_map(|i| pixel(i % size, i / size, size, base))
        .collect();
    let mut s = size;
    loop {
        levels.push(current.clone());
        if s == 1 {
            break;
        }
        let half = s / 2;
        let mut next = vec![0u8; (half * half * 4) as usize];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |dx: u32, dy: u32| {
                        current[(((y * 2 + dy) * s + x * 2 + dx) * 4 + c) as usize] as u32
                    };
                    next[((y * half + x) * 4 + c) as usize] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                }
            }
        }
        current = next;
        s = half;
    }
    Ktx2Texture::new_2d(size, size, TextureFormat::Rgba8Srgb, levels)
}

/// Writes the pack. Textures go in uncompressed so their mips can be read by range.
pub fn build_pak(path: &std::path::Path) -> Result<(), PakError> {
    let mut pak = PakWriter::new();
    pak.add(LEVEL, LEVEL_TEXT.as_bytes().to_vec())?;
    pak.add_uncompressed(
        GROUND,
        texture(2048, [0.30, 0.50, 0.26]).to_ktx2_bytes(false),
    )?;
    pak.add_uncompressed(CRATE, texture(256, [0.62, 0.42, 0.24]).to_ktx2_bytes(false))?;
    pak.write_to(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_in_the_pack_parses_and_has_reachable_coins() {
        let level = crate::game::parse_level(LEVEL_TEXT).unwrap();
        assert_eq!(level.coins.len(), 8);
        for (x, z) in &level.coins {
            assert!(x.abs() < level.half && z.abs() < level.half);
            // No coin is buried inside a crate.
            assert!(
                level
                    .crates
                    .iter()
                    .all(|c| (x - c.x).abs() > c.half + 0.3 || (z - c.z).abs() > c.half + 0.3)
            );
        }
    }

    #[test]
    fn textures_have_full_mip_chains() {
        let t = texture(64, [0.3, 0.5, 0.3]);
        assert_eq!(t.levels.len(), 7);
        assert_eq!(t.levels[0].len(), 64 * 64 * 4);
        assert_eq!(t.levels[6].len(), 4);
        let again = redox_asset::Ktx2Loader::default();
        let _ = again;
    }

    #[test]
    fn the_pack_round_trips() {
        use redox_asset::{AssetSource, PakSource};
        let path = std::env::temp_dir().join(format!("courier_test_{}.pak", std::process::id()));
        build_pak(&path).unwrap();
        let source = PakSource::open(&path).unwrap();
        assert!(source.read(LEVEL).unwrap().starts_with(b"# Courier"));
        let ground = redox_asset::StreamedTexture::open(
            std::sync::Arc::new(PakSource::open(&path).unwrap()),
            GROUND,
            Default::default(),
        )
        .unwrap();
        assert_eq!((ground.info().width, ground.info().mips), (2048, 12));
        let _ = std::fs::remove_file(path);
    }
}

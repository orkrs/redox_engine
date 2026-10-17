//! Loads KTX2 files made by other tools (set `REDOX_THIRD_PARTY_KTX2` to a folder of them).
//! Without the variable the test does nothing.

#![allow(clippy::unwrap_used)]

use redox_asset::{AssetLoader, Ktx2Loader};

#[test]
fn third_party_ktx2_files_load() {
    let Some(dir) = std::env::var_os("REDOX_THIRD_PARTY_KTX2") else {
        return;
    };
    let mut loaded = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("ktx2") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let result = Ktx2Loader::default().load(&bytes);
        match &result {
            Ok(t) => {
                let distinct = t.levels[0]
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                eprintln!(
                    "{}: {}x{} {:?} layers {} faces {} mips {} distinct bytes in mip0 {}",
                    path.file_name().unwrap().to_string_lossy(),
                    t.width,
                    t.height,
                    t.format,
                    t.layers,
                    t.faces,
                    t.levels.len(),
                    distinct
                );
                assert!(distinct > 1, "{path:?} decoded to a flat image");
            }
            Err(e) => eprintln!("{}: ERROR {e}", path.file_name().unwrap().to_string_lossy()),
        }
        result.unwrap();
        loaded += 1;
    }
    assert!(loaded > 0, "no .ktx2 files in the folder");
}

/// The same picture packed as BasisLZ (ETC1S) and as UASTC+Zstd must decode to nearly the same pixels.
#[test]
fn etc1s_and_uastc_versions_of_one_picture_agree() {
    let Some(dir) = std::env::var_os("REDOX_THIRD_PARTY_KTX2") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let (Ok(a), Ok(b)) = (
        std::fs::read(dir.join("color_grid_blze.ktx2")),
        std::fs::read(dir.join("color_grid_uastc_zstd_5.ktx2")),
    ) else {
        return;
    };
    let a = Ktx2Loader::default().load(&a).unwrap();
    let b = Ktx2Loader::default().load(&b).unwrap();
    assert_eq!((a.width, a.height), (b.width, b.height));
    let error = a.levels[0]
        .iter()
        .zip(&b.levels[0])
        .map(|(x, y)| (*x as f64 - *y as f64).abs())
        .sum::<f64>()
        / a.levels[0].len() as f64;
    eprintln!("ETC1S vs UASTC mean error: {error:.2}");
    assert!(error < 12.0, "decoders disagree about the picture: {error}");
}

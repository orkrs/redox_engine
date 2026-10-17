//! GUID sidecars, the registry, and `.pak` archives against real temp directories.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use redox_asset::{
    AssetGuid, AssetRegistry, AssetSource, DirSource, PakError, PakReader, PakSource, PakWriter,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("redox_asset_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn guid_text_form_round_trips_and_rejects_garbage() {
    let g = AssetGuid(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
    let text = g.to_string();
    assert_eq!(text, "0123456789abcdef0011223344556677");
    assert_eq!(text.parse::<AssetGuid>().unwrap(), g);
    for bad in ["", "xyz", "0123", &"g".repeat(32), &"a".repeat(33)] {
        assert!(bad.parse::<AssetGuid>().is_err(), "{bad:?}");
    }
    assert_ne!(AssetGuid::new_random(), AssetGuid::new_random());
}

#[test]
fn scan_creates_sidecars_and_is_stable_across_runs() {
    let dir = TempDir::new("scan");
    dir.write("tex/a.png", b"aaa");
    dir.write("mesh/b.glb", b"bbb");
    let (reg, report) = AssetRegistry::scan(&dir.0).unwrap();
    assert_eq!(reg.len(), 2);
    assert_eq!(report.created.len(), 2);
    assert!(dir.0.join("tex/a.png.meta").exists());

    let guid = reg.guid_of("tex/a.png").unwrap();
    assert_eq!(reg.path_of(guid), Some("tex/a.png"));

    let (again, report2) = AssetRegistry::scan(&dir.0).unwrap();
    assert!(report2.created.is_empty(), "sidecars already exist");
    assert_eq!(
        again.guid_of("tex/a.png"),
        Some(guid),
        "GUIDs are persistent"
    );
}

#[test]
fn moving_an_asset_keeps_its_guid() {
    let dir = TempDir::new("move");
    dir.write("a.png", b"data");
    let (mut reg, _) = AssetRegistry::scan(&dir.0).unwrap();
    let guid = reg.guid_of("a.png").unwrap();

    assert_eq!(reg.move_asset("a.png", "sub/dir/b.png").unwrap(), guid);
    assert!(!dir.0.join("a.png").exists());
    assert!(dir.0.join("sub/dir/b.png").exists());
    assert!(dir.0.join("sub/dir/b.png.meta").exists());
    assert_eq!(reg.path_of(guid), Some("sub/dir/b.png"));

    let (fresh, _) = AssetRegistry::scan(&dir.0).unwrap();
    assert_eq!(fresh.guid_of("sub/dir/b.png"), Some(guid));

    assert!(reg.move_asset("missing.png", "x.png").is_err());
    dir.write("taken.png", b"x");
    let (mut reg, _) = AssetRegistry::scan(&dir.0).unwrap();
    assert!(reg.move_asset("sub/dir/b.png", "taken.png").is_err());
}

#[test]
fn copied_sidecars_get_a_fresh_guid() {
    let dir = TempDir::new("dup");
    dir.write("a.png", b"data");
    let (reg, _) = AssetRegistry::scan(&dir.0).unwrap();
    let original = reg.guid_of("a.png").unwrap();

    // Copy file and sidecar, as a file manager would.
    std::fs::copy(dir.0.join("a.png"), dir.0.join("b.png")).unwrap();
    std::fs::copy(dir.0.join("a.png.meta"), dir.0.join("b.png.meta")).unwrap();

    let (reg, report) = AssetRegistry::scan(&dir.0).unwrap();
    assert_eq!(report.regenerated, [Path::new("b.png")]);
    assert_eq!(
        reg.guid_of("a.png"),
        Some(original),
        "the first path keeps its GUID"
    );
    assert_ne!(reg.guid_of("b.png"), Some(original));
    assert_eq!(reg.len(), 2);
}

#[test]
fn damaged_sidecars_are_reported_not_fatal() {
    let dir = TempDir::new("damaged");
    dir.write("ok.png", b"1");
    dir.write("bad.png", b"2");
    dir.write("bad.png.meta", b"this is not ron");
    let (reg, report) = AssetRegistry::scan(&dir.0).unwrap();
    assert_eq!(report.damaged, [Path::new("bad.png")]);
    assert_eq!(reg.len(), 1);
}

#[test]
fn modification_is_detected_via_the_recorded_hash() {
    let dir = TempDir::new("hash");
    dir.write("a.txt", b"v1");
    let (reg, _) = AssetRegistry::scan(&dir.0).unwrap();
    assert!(!reg.is_modified("a.txt").unwrap());
    dir.write("a.txt", b"v2");
    assert!(reg.is_modified("a.txt").unwrap());
    reg.mark_imported("a.txt", 3).unwrap();
    assert!(!reg.is_modified("a.txt").unwrap());
}

#[test]
fn remove_deletes_file_and_sidecar() {
    let dir = TempDir::new("remove");
    dir.write("a.png", b"1");
    let (mut reg, _) = AssetRegistry::scan(&dir.0).unwrap();
    let guid = reg.guid_of("a.png").unwrap();
    reg.remove_asset("a.png").unwrap();
    assert!(!dir.0.join("a.png").exists() && !dir.0.join("a.png.meta").exists());
    assert!(reg.path_of(guid).is_none());
    assert!(reg.remove_asset("a.png").is_err());
}

#[test]
fn pak_round_trip_and_directory_source_agree() {
    let dir = TempDir::new("pak");
    dir.write("data/level.ron", b"(level: 1)");
    let pak_path = dir.0.join("game.pak");

    let mut writer = PakWriter::new();
    writer.add_dir(&dir.0.join("data")).unwrap();
    writer.add("tex/a.png", b"PNG-bytes".to_vec()).unwrap();
    assert_eq!(writer.len(), 2);
    writer.write_to(&pak_path).unwrap();

    let source = PakSource::open(&pak_path).unwrap();
    assert!(source.exists("tex/a.png") && source.exists("level.ron"));
    assert!(!source.exists("missing"));
    assert_eq!(source.read("tex/a.png").unwrap(), b"PNG-bytes");
    assert_eq!(
        source.read("nope").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );

    let folder = DirSource::new(dir.0.join("data"));
    assert_eq!(
        folder.read("level.ron").unwrap(),
        source.read("level.ron").unwrap()
    );
    assert!(folder.read("../game.pak").is_err(), "traversal is rejected");
    assert!(!folder.exists("/etc/passwd"));
}

#[test]
fn packing_is_deterministic() {
    let build = |order: &[&str]| {
        let mut w = PakWriter::new();
        for name in order {
            w.add(name, name.as_bytes().to_vec()).unwrap();
        }
        w.to_bytes()
    };
    assert_eq!(build(&["b", "a", "c/d"]), build(&["c/d", "a", "b"]));
}

#[test]
fn unsafe_entry_names_are_rejected() {
    let mut w = PakWriter::new();
    for bad in [
        "", "../x", "a/../b", "/abs", "a//b", r"a\b", "c:evil", "./x",
    ] {
        assert!(
            matches!(w.add(bad, vec![]), Err(PakError::InvalidName(_))),
            "{bad:?}"
        );
    }
    assert!(w.add("ok/name.bin", vec![1]).is_ok());
}

#[test]
fn corruption_is_detected() {
    let dir = TempDir::new("corrupt");
    let mut w = PakWriter::new();
    w.add("a.bin", vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    let bytes = w.to_bytes();
    let path = dir.0.join("x.pak");

    // Flipped content byte: opens fine, reading fails the hash check.
    let mut flipped = bytes.clone();
    flipped[24] ^= 0xFF;
    std::fs::write(&path, &flipped).unwrap();
    let reader = PakReader::open(&path).unwrap();
    assert!(matches!(reader.read("a.bin"), Err(PakError::Corrupt(_))));

    // Truncations at every length either fail to open or fail to read, never panic.
    for cut in 0..bytes.len() {
        std::fs::write(&path, &bytes[..cut]).unwrap();
        if let Ok(r) = PakReader::open(&path) {
            let _ = r.read("a.bin");
        }
    }

    // Bad magic and missing file.
    let mut bad = bytes.clone();
    bad[0] = 0;
    std::fs::write(&path, &bad).unwrap();
    assert!(PakReader::open(&path).is_err());
    assert!(PakReader::open(&dir.0.join("missing.pak")).is_err());
}

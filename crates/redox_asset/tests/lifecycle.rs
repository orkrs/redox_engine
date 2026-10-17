//! Asset ownership (ref counts, dependencies), loading from `.pak` and compressed archives.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use redox_asset::{
    AssetError, AssetLoader, AssetManager, AssetStatus, PakError, PakReader, PakSource, PakWriter,
};
use redox_ecs::World;

struct TextLoader;

impl AssetLoader for TextLoader {
    type Asset = String;

    fn load(&self, bytes: &[u8]) -> Result<String, AssetError> {
        String::from_utf8(bytes.to_vec()).map_err(|e| AssetError::LoadFailed(e.to_string()))
    }

    fn extensions(&self) -> Vec<&'static str> {
        vec!["txt"]
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("redox_life_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn manager() -> AssetManager {
    let mut mgr = AssetManager::new(".");
    mgr.register_loader::<String, _>(TextLoader);
    mgr
}

fn pump_until(mgr: &mut AssetManager, world: &mut World, done: impl Fn(&AssetManager) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(mgr) {
        assert!(Instant::now() < deadline, "timed out waiting for loads");
        mgr.update(world);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn pak_with(entries: &[(&str, &str)], compress: bool, dir: &TempDir) -> PakSource {
    let mut writer = PakWriter::new();
    writer.set_compression(compress);
    for (name, text) in entries {
        writer.add(name, text.as_bytes().to_vec()).unwrap();
    }
    let path = dir.0.join("data.pak");
    writer.write_to(&path).unwrap();
    PakSource::open(&path).unwrap()
}

#[test]
fn release_unloads_at_zero_and_cached_loads_count_as_owners() {
    let dir = TempDir::new("refs");
    let mut mgr = manager();
    mgr.set_source(Arc::new(pak_with(&[("a.txt", "alpha")], false, &dir)));
    let mut world = World::new();

    let first = mgr.load_async::<String>("a.txt").unwrap();
    let second = mgr.load_async::<String>("a.txt").unwrap();
    assert_eq!(first, second, "same path, same handle");
    assert_eq!(mgr.ref_count(first.id()), 2);
    pump_until(&mut mgr, &mut world, |m| {
        m.status(first) == AssetStatus::Ready
    });
    assert_eq!(mgr.get(first).unwrap(), "alpha");

    assert!(!mgr.release(first));
    assert_eq!(mgr.get(first).unwrap(), "alpha");
    assert!(mgr.release(second));
    assert!(mgr.get(first).is_none());
    assert_eq!(mgr.status(first), AssetStatus::Failed);
    assert_eq!(mgr.live_assets(), 0);
    assert!(!mgr.release(first), "double release is harmless");

    // The path cache was cleared: loading again starts a fresh load.
    let again = mgr.load_async::<String>("a.txt").unwrap();
    assert_ne!(again, first);
    pump_until(&mut mgr, &mut world, |m| {
        m.status(again) == AssetStatus::Ready
    });
}

#[test]
fn retain_delays_unloading() {
    let mut mgr = manager();
    let handle = mgr.insert("kept".to_string());
    mgr.retain(handle);
    assert!(!mgr.release(handle));
    assert!(mgr.release(handle));
    assert!(mgr.get(handle).is_none());
}

#[test]
fn dependencies_live_as_long_as_their_parent() {
    let mut mgr = manager();
    let child = mgr.insert("texture".to_string());
    let parent = mgr.insert("material".to_string());
    assert!(mgr.add_dependency(parent.id(), child.id()));
    assert!(mgr.add_dependency(parent.id(), child.id()), "idempotent");
    assert_eq!(mgr.ref_count(child.id()), 2);
    assert_eq!(mgr.dependencies(parent.id()), [child.id()]);
    assert_eq!(
        mgr.status_with_dependencies(parent.id()),
        AssetStatus::Ready
    );

    // The scene drops its own reference to the child; the parent still holds it.
    assert!(!mgr.release(child));
    assert_eq!(mgr.get(child).unwrap(), "texture");
    // Dropping the parent releases the child too.
    assert!(mgr.release(parent));
    assert!(mgr.get(child).is_none());
    assert_eq!(mgr.live_assets(), 0);
}

#[test]
fn dependency_status_reflects_loading_and_unknown_ids() {
    let dir = TempDir::new("depstatus");
    let mut mgr = manager();
    mgr.set_source(Arc::new(pak_with(&[("c.txt", "child")], false, &dir)));
    let mut world = World::new();
    let parent = mgr.insert("parent".to_string());
    let child = mgr.load_async::<String>("c.txt").unwrap();
    mgr.add_dependency(parent.id(), child.id());
    assert_eq!(
        mgr.status_with_dependencies(parent.id()),
        AssetStatus::Loading
    );
    pump_until(&mut mgr, &mut world, |m| {
        m.status_with_dependencies(parent.id()) == AssetStatus::Ready
    });
    let unknown = redox_asset::AssetId(u64::MAX);
    assert_eq!(mgr.status_with_dependencies(unknown), AssetStatus::Failed);
    assert!(!mgr.add_dependency(parent.id(), unknown));
}

#[test]
fn unloading_during_a_load_drops_the_result() {
    let dir = TempDir::new("inflight");
    let mut mgr = manager();
    mgr.set_source(Arc::new(pak_with(&[("x.txt", "late")], false, &dir)));
    let mut world = World::new();
    let handle = mgr.load_async::<String>("x.txt").unwrap();
    assert!(mgr.release(handle));
    std::thread::sleep(Duration::from_millis(100));
    mgr.update(&mut world);
    assert!(mgr.get(handle).is_none(), "no ghost asset");
    assert_eq!(mgr.live_assets(), 0);
}

#[test]
fn completion_budget_spreads_loads_over_updates() {
    let dir = TempDir::new("budget");
    let names: Vec<String> = (0..6).map(|i| format!("f{i}.txt")).collect();
    let entries: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "x")).collect();
    let mut mgr = manager();
    mgr.set_source(Arc::new(pak_with(&entries, false, &dir)));
    mgr.set_completion_budget(2);
    let mut world = World::new();
    let handles: Vec<_> = names
        .iter()
        .map(|n| mgr.load_async::<String>(n).unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(300)); // everything has finished loading

    let ready = |m: &AssetManager| {
        handles
            .iter()
            .filter(|h| m.status(**h) == AssetStatus::Ready)
            .count()
    };
    mgr.update(&mut world);
    assert_eq!(ready(&mgr), 2);
    mgr.update(&mut world);
    assert_eq!(ready(&mgr), 4);
    mgr.update(&mut world);
    assert_eq!(ready(&mgr), 6);
}

#[test]
fn missing_entries_in_a_pak_fail_the_load() {
    let dir = TempDir::new("missing");
    let mut mgr = manager();
    mgr.set_source(Arc::new(pak_with(&[("a.txt", "a")], false, &dir)));
    let mut world = World::new();
    let handle = mgr.load_async::<String>("nope.txt").unwrap();
    pump_until(&mut mgr, &mut world, |m| {
        m.status(handle) == AssetStatus::Failed
    });
}

#[test]
fn compressed_archives_round_trip_and_shrink() {
    let dir = TempDir::new("lz4");
    let text = "the quick brown fox ".repeat(500);
    let noise: Vec<u8> = (0..64u32)
        .map(|i| (i.wrapping_mul(2_654_435_761u32) >> 24) as u8)
        .collect();

    let build = |compress: bool| {
        let mut w = PakWriter::new();
        w.set_compression(compress);
        w.add("text.txt", text.clone().into_bytes()).unwrap();
        w.add("noise.bin", noise.clone()).unwrap();
        w.add("empty.bin", Vec::new()).unwrap();
        w.to_bytes()
    };
    let plain = build(false);
    let packed = build(true);
    assert!(
        packed.len() < plain.len() / 2,
        "{} vs {}",
        packed.len(),
        plain.len()
    );
    assert_eq!(packed, build(true), "compressed output is deterministic");

    let path = dir.0.join("c.pak");
    std::fs::write(&path, &packed).unwrap();
    let reader = PakReader::open(&path).unwrap();
    assert_eq!(reader.read("text.txt").unwrap(), text.as_bytes());
    assert_eq!(reader.read("noise.bin").unwrap(), noise);
    assert!(reader.read("empty.bin").unwrap().is_empty());
}

#[test]
fn damaged_compressed_entries_are_rejected_without_panics() {
    let dir = TempDir::new("lz4bad");
    let mut w = PakWriter::new();
    w.set_compression(true);
    w.add("t.txt", "abcabcabc".repeat(100).into_bytes())
        .unwrap();
    let bytes = w.to_bytes();
    let path = dir.0.join("bad.pak");

    // Flip every byte in turn: open or read must fail cleanly, never panic.
    for i in 0..bytes.len() {
        let mut broken = bytes.clone();
        broken[i] ^= 0xA5;
        std::fs::write(&path, &broken).unwrap();
        if let Ok(reader) = PakReader::open(&path)
            && let Ok(data) = reader.read("t.txt")
        {
            assert_eq!(data, "abcabcabc".repeat(100).as_bytes(), "byte {i}");
        }
    }
    // A tiny archive must not be able to demand a huge allocation.
    let mut bomb = bytes.clone();
    let raw_size_at = bomb.len() - 9;
    bomb[raw_size_at..raw_size_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(&path, &bomb).unwrap();
    assert!(matches!(PakReader::open(&path), Err(PakError::Corrupt(_))));
}

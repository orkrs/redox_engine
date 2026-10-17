//! Streaming textures: header-only open, per-mip range reads, priorities and byte budgets.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use redox_asset::testing::{Ktx2Builder, Supercompression};
use redox_asset::{
    AssetManager, AssetSource, AssetStatus, DirSource, LoadOptions, PakSource, PakWriter, Priority,
    StreamedTexture, TextureFormat,
};
use redox_ecs::World;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("redox_stream_{tag}_{}", std::process::id()));
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

fn pump_until(mgr: &mut AssetManager, world: &mut World, done: impl Fn(&AssetManager) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(mgr) {
        assert!(Instant::now() < deadline, "timed out");
        mgr.update(world);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn texture_builders() -> Vec<(&'static str, Ktx2Builder)> {
    vec![
        ("plain", Ktx2Builder::new(TextureFormat::Bc7Unorm, 64, 32)),
        (
            "zstd",
            Ktx2Builder::new(TextureFormat::Rgba8Unorm, 32, 32)
                .supercompression(Supercompression::Zstd),
        ),
        (
            "cube",
            Ktx2Builder::new(TextureFormat::Rgba8Unorm, 16, 16).cube(),
        ),
        (
            "array",
            Ktx2Builder::new(TextureFormat::Bc7Unorm, 16, 8).layers(3),
        ),
    ]
}

#[test]
fn every_mip_of_a_ktx2_file_can_be_read_alone_from_a_folder() {
    let dir = TempDir::new("dir");
    let source: Arc<dyn AssetSource> = Arc::new(DirSource::new(&dir.0));
    for (name, builder) in texture_builders() {
        std::fs::write(dir.0.join(format!("{name}.ktx2")), builder.build()).unwrap();
        let tex = StreamedTexture::open(
            source.clone(),
            &format!("{name}.ktx2"),
            LoadOptions::default(),
        )
        .unwrap();
        let info = *tex.info();
        assert_eq!(info.mips as usize, builder.raw_levels().len(), "{name}");
        for (mip, expected) in builder.raw_levels().iter().enumerate() {
            assert_eq!(
                info.mip_bytes(mip as u32),
                expected.len(),
                "{name} mip {mip}"
            );
            assert_eq!(
                &tex.read_mip(mip as u32).unwrap(),
                expected,
                "{name} mip {mip}"
            );
        }
        assert!(tex.read_mip(info.mips).is_err());
        assert_eq!(
            info.bytes_from(0),
            builder.raw_levels().iter().map(Vec::len).sum::<usize>()
        );
    }
}

#[test]
fn pak_entries_stream_raw_and_compressed_alike() {
    let builder = Ktx2Builder::new(TextureFormat::Bc7Unorm, 64, 64);
    let file = builder.build();
    let dir = TempDir::new("pak");
    let mut w = PakWriter::new();
    w.set_compression(true);
    w.add_uncompressed("raw.ktx2", file.clone()).unwrap();
    w.add("lz4.ktx2", file).unwrap();
    let path = dir.0.join("t.pak");
    w.write_to(&path).unwrap();
    let source: Arc<dyn AssetSource> = Arc::new(PakSource::open(&path).unwrap());
    for name in ["raw.ktx2", "lz4.ktx2"] {
        let tex = StreamedTexture::open(source.clone(), name, LoadOptions::default()).unwrap();
        for (mip, expected) in builder.raw_levels().iter().enumerate() {
            assert_eq!(
                &tex.read_mip(mip as u32).unwrap(),
                expected,
                "{name} mip {mip}"
            );
        }
    }
}

#[test]
fn sources_support_range_reads_and_reject_bad_ranges() {
    let dir = TempDir::new("range");
    std::fs::write(dir.0.join("a.bin"), (0..100u8).collect::<Vec<_>>()).unwrap();
    let mut w = PakWriter::new();
    w.add_uncompressed("a.bin", (0..100u8).collect()).unwrap();
    let pak = dir.0.join("r.pak");
    w.write_to(&pak).unwrap();
    let sources: [Arc<dyn AssetSource>; 2] = [
        Arc::new(DirSource::new(&dir.0)),
        Arc::new(PakSource::open(&pak).unwrap()),
    ];
    for source in sources {
        assert_eq!(source.size("a.bin").unwrap(), 100);
        assert_eq!(
            source.read_range("a.bin", 10, 5).unwrap(),
            [10, 11, 12, 13, 14]
        );
        assert_eq!(source.read_range("a.bin", 95, 5).unwrap().len(), 5);
        assert!(source.read_range("a.bin", 96, 5).is_err());
        assert!(source.read_range("a.bin", u64::MAX, 1).is_err());
        assert!(source.read_range("missing.bin", 0, 1).is_err());
    }
}

#[test]
fn basis_files_stream_as_a_whole_with_the_requested_target() {
    let dir = TempDir::new("basis");
    std::fs::write(
        dir.0.join("logo.basis"),
        include_bytes!("data/rust-logo-uastc.basis"),
    )
    .unwrap();
    let source: Arc<dyn AssetSource> = Arc::new(DirSource::new(&dir.0));
    let tex = StreamedTexture::open(source, "logo.basis", LoadOptions::default()).unwrap();
    assert_eq!((tex.info().width, tex.info().mips), (64, 7));
    assert_eq!(tex.info().format, TextureFormat::Rgba8Srgb);
    assert_eq!(tex.read_mip(0).unwrap().len(), 64 * 64 * 4);
}

#[test]
fn damaged_headers_and_level_indexes_fail_cleanly() {
    let dir = TempDir::new("bad");
    let source: Arc<dyn AssetSource> = Arc::new(DirSource::new(&dir.0));
    let good = Ktx2Builder::new(TextureFormat::Bc7Unorm, 32, 32).build();
    for i in 0..good.len().min(200) {
        for flip in [0x01u8, 0x80, 0xFF] {
            let mut broken = good.clone();
            broken[i] ^= flip;
            std::fs::write(dir.0.join("b.ktx2"), &broken).unwrap();
            if let Ok(tex) = StreamedTexture::open(source.clone(), "b.ktx2", LoadOptions::default())
            {
                for mip in 0..tex.info().mips {
                    let _ = tex.read_mip(mip);
                }
            }
        }
    }
    std::fs::write(dir.0.join("b.ktx2"), &good[..40]).unwrap();
    assert!(StreamedTexture::open(source.clone(), "b.ktx2", LoadOptions::default()).is_err());
    assert!(StreamedTexture::open(source, "nope.ktx2", LoadOptions::default()).is_err());
}

#[test]
fn manager_opens_headers_reads_mips_by_priority_and_budgets_the_result() {
    let dir = TempDir::new("manager");
    let builder = Ktx2Builder::new(TextureFormat::Rgba8Unorm, 64, 64);
    std::fs::write(dir.0.join("t.ktx2"), builder.build()).unwrap();
    let mut mgr = AssetManager::with_workers(&dir.0, 1);
    let mut world = World::new();

    let handle = mgr.stream_texture("t.ktx2", Priority::HIGH).unwrap();
    assert_eq!(mgr.stream_status(handle), AssetStatus::Loading);
    pump_until(&mut mgr, &mut world, |m| {
        m.stream_status(handle) == AssetStatus::Ready
    });
    let info = mgr.stream_info(handle).unwrap();
    assert_eq!((info.width, info.mips), (64, 7));
    // Asking for the same path again shares the handle and adds an owner.
    assert_eq!(mgr.stream_texture("t.ktx2", Priority::LOW).unwrap(), handle);
    assert_eq!(mgr.ref_count(handle.id()), 2);

    assert!(mgr.request_mip(handle, 3, Priority::NORMAL));
    assert!(
        !mgr.request_mip(handle, 3, Priority::HIGH),
        "already requested"
    );
    assert!(!mgr.request_mip(handle, 99, Priority::HIGH), "no such mip");
    for mip in [0, 1, 2, 4, 5, 6] {
        assert!(mgr.request_mip(handle, mip, Priority::NORMAL));
    }
    assert_eq!(mgr.pending_mips(), 7);

    // Collect everything; the byte budget limits how much one call returns.
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while seen.len() < 7 {
        assert!(Instant::now() < deadline, "mips never arrived");
        let batch = mgr.drain_mips(5_000);
        let bytes: usize = batch.iter().map(|m| m.data.as_ref().unwrap().len()).sum();
        assert!(
            batch.len() <= 1 || bytes <= 5_000,
            "budget exceeded: {bytes}"
        );
        seen.extend(batch);
        std::thread::sleep(Duration::from_millis(2));
    }
    for ready in &seen {
        assert_eq!(ready.texture, handle.id());
        assert_eq!(
            &ready.data.as_ref().unwrap(),
            &&builder.raw_levels()[ready.mip as usize]
        );
    }
    assert_eq!(mgr.pending_mips(), 0);
}

#[test]
fn unloading_a_texture_drops_its_queued_and_finished_reads() {
    let dir = TempDir::new("unload");
    std::fs::write(
        dir.0.join("t.ktx2"),
        Ktx2Builder::new(TextureFormat::Rgba8Unorm, 32, 32).build(),
    )
    .unwrap();
    let mut mgr = AssetManager::with_workers(&dir.0, 1);
    let mut world = World::new();
    let handle = mgr.stream_texture("t.ktx2", Priority::HIGH).unwrap();
    pump_until(&mut mgr, &mut world, |m| {
        m.stream_status(handle) == AssetStatus::Ready
    });
    for mip in 0..6 {
        mgr.request_mip(handle, mip, Priority::NORMAL);
    }
    assert!(mgr.release(handle));
    assert_eq!(mgr.pending_mips(), 0, "requests were forgotten");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        mgr.drain_mips(usize::MAX).is_empty(),
        "finished reads of a dead texture are dropped"
    );
    assert!(!mgr.request_mip(handle, 0, Priority::HIGH));
}

#[test]
fn a_missing_file_marks_the_handle_failed() {
    let dir = TempDir::new("missing");
    let mut mgr = AssetManager::with_workers(&dir.0, 1);
    let mut world = World::new();
    let handle = mgr.stream_texture("absent.ktx2", Priority::NORMAL).unwrap();
    pump_until(&mut mgr, &mut world, |m| {
        m.stream_status(handle) == AssetStatus::Failed
    });
}

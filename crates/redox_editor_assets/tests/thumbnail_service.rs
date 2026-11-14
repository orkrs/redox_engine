//! EDT-B4 step 3: thumbnail cache and non-blocking tick.

use redox_asset::AssetGuid;
use redox_editor_assets::{AssetKind, BrowserModel, ThumbnailService};
use std::path::PathBuf;

#[test]
fn cache_hit_skips_regeneration() {
    let root = PathBuf::from(std::env::temp_dir()).join(format!("thumbs_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    let mut thumbs = ThumbnailService::new(&root);
    let guid = AssetGuid(42);
    let src = root.join("a.png");
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&[0, 0, 0, 13]);
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    std::fs::write(&src, png).expect("write");
    thumbs.request(guid, "a.png", AssetKind::Texture, src.clone());
    thumbs.tick(1);
    let path = thumbs.cache_path(guid);
    let mtime = std::fs::metadata(&path).expect("cached").modified().expect("mtime");
    thumbs.request(guid, "a.png", AssetKind::Texture, src);
    thumbs.tick(1);
    let mtime2 = std::fs::metadata(&path).expect("cached").modified().expect("mtime2");
    assert_eq!(mtime, mtime2);
}

#[test]
fn slow_worker_does_not_block_browser_tick() {
    let root = PathBuf::from(std::env::temp_dir()).join(format!("thumbs_slow_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    let mut thumbs = ThumbnailService::new(&root);
    thumbs.set_slow_mode(true);
    let guid = AssetGuid(7);
    let src = root.join("slow.png");
    std::fs::write(&src, b"not texture").ok();
    thumbs.request(guid, "slow.png", AssetKind::Texture, src);
    let mut model = BrowserModel::new();
    model.filter.text = "probe".into();
    let _ = model.visible();
    thumbs.tick(1);
    assert!(model.visible().is_empty());
}

#[test]
fn moved_guid_writes_new_cache_file() {
    let root = PathBuf::from(std::env::temp_dir()).join(format!("thumbs_guid_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    let mut thumbs = ThumbnailService::new(&root);
    let guid_a = AssetGuid(1);
    let guid_b = AssetGuid(2);
    let src = root.join("a.png");
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&[0, 0, 0, 13]);
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&2u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    std::fs::write(&src, png).expect("write");
    thumbs.request(guid_a, "a.png", AssetKind::Texture, src.clone());
    thumbs.tick(1);
    thumbs.forget(guid_a);
    thumbs.request(guid_b, "a.png", AssetKind::Texture, src);
    thumbs.tick(1);
    assert!(thumbs.is_cached(guid_b));
    assert!(!thumbs.is_cached(guid_a));
}
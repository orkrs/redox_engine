//! GPU memory accounting as the renderer reports it (plan item A4, wiring).
//!
//! `Renderer::memory_usage()` builds a `MemoryBudget` from the live resources (meshes, textures, shadow
//! maps, virtual geometry buffers, render targets under `Other`) so nothing can be missed on unload.
//! `set_memory_limit` stores limits that `memory_usage` and `memory_report` carry along.
//!
//! Tests are skipped when no GPU adapter exists.

#![allow(clippy::unwrap_used)]

mod common;

use redox_render::mesh::primitive;
use redox_render::{MemoryCategory, Renderer};

fn renderer() -> Option<Renderer> {
    let adapter = common::adapter()?;
    common::renderer(&adapter, 64)
}

fn png(size: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(size, size, image::Rgba([200, 90, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("PNG encoding into memory");
    out.into_inner()
}

#[test]
fn a_fresh_renderer_accounts_shadow_maps_and_render_targets() {
    let Some(r) = renderer() else { return };
    let usage = r.memory_usage();
    assert!(usage.used(MemoryCategory::Shadows) > 0, "shadow atlases");
    assert!(usage.used(MemoryCategory::Other) > 0, "render targets");
    assert_eq!(usage.total_used(), r.memory_usage().total_used(), "stable");
}

#[test]
fn uploading_a_mesh_adds_its_buffers() {
    let Some(mut r) = renderer() else { return };
    let before = r.memory_usage().used(MemoryCategory::Meshes);
    let mesh = primitive::create_cube();
    let slot = r.upload_mesh(&mesh);
    let vertex_bytes = r.read_mesh_vertex_bytes(slot).unwrap().len() as u64;
    let index_bytes = mesh.indices.len() as u64 * 4;
    let after = r.memory_usage().used(MemoryCategory::Meshes);
    assert_eq!(after - before, vertex_bytes + index_bytes);
}

#[test]
fn uploading_a_texture_adds_between_one_level_and_a_full_chain() {
    let Some(mut r) = renderer() else { return };
    let before = r.memory_usage().used(MemoryCategory::Textures);
    r.upload_texture(&png(64), "wiring_test").unwrap();
    let added = r.memory_usage().used(MemoryCategory::Textures) - before;
    let one_level = 64 * 64 * 4;
    assert!(added >= one_level, "{added}");
    assert!(
        added <= one_level * 4 / 3 + 64,
        "at most a full mip chain: {added}"
    );
}

#[test]
fn limits_are_carried_into_the_usage_and_the_report() {
    let Some(mut r) = renderer() else { return };
    assert_eq!(r.memory_usage().limit(MemoryCategory::Textures), None);
    r.set_memory_limit(MemoryCategory::Textures, Some(1));
    let usage = r.memory_usage();
    assert_eq!(usage.limit(MemoryCategory::Textures), Some(1));
    assert!(
        usage.is_over_budget(MemoryCategory::Textures),
        "fallback textures exist"
    );
    let report = r.memory_report();
    assert!(
        report.contains("textures:") && report.contains("OVER"),
        "{report}"
    );
    assert!(report.lines().last().unwrap().starts_with("total:"));
    r.set_memory_limit(MemoryCategory::Textures, None);
    assert!(!r.memory_report().contains("OVER"));
}

#[test]
fn a_fresh_renderer_accounts_its_persistent_buffers() {
    let Some(r) = renderer() else { return };
    let usage = r.memory_usage();
    assert!(
        usage.used(MemoryCategory::Buffers) >= 1024,
        "camera, light and model buffers"
    );
    assert_eq!(
        usage.used(MemoryCategory::Buffers),
        r.memory_usage().used(MemoryCategory::Buffers),
        "stable between calls"
    );
}

#[test]
fn the_report_names_the_buffers_line() {
    let Some(r) = renderer() else { return };
    assert!(
        r.memory_report().contains("buffers:"),
        "{}",
        r.memory_report()
    );
}

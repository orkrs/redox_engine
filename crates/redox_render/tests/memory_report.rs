//! Specification (tests first) for the pure part of wiring `MemoryBudget` into the renderer (plan item
//! A4). No GPU. Everything goes into `crates/redox_render/src/memory_budget.rs` and is re-exported from
//! the crate root next to `MemoryBudget`.
//!
//! ```text
//! /// Bytes of a 2D texture: sum over `mip_levels` levels of max(w >> level, 1) * max(h >> level, 1)
//! /// * bytes_per_pixel. `mip_levels` 0 counts as 1. Saturating, never panics.
//! pub fn texture_size_bytes(width: u32, height: u32, bytes_per_pixel: u32, mip_levels: u32) -> u64;
//!
//! impl MemoryBudget {
//!     /// The categories whose usage exceeds their limit, in `MemoryCategory::ALL` order.
//!     pub fn over_budget_categories(&self) -> Vec<MemoryCategory>;
//!     /// Text for a debug overlay, one line per category that has usage or a limit, in `ALL` order,
//!     /// then a `total:` line. Sizes are in MiB with one decimal (1 MiB = 1048576 bytes).
//!     ///   limited:    "textures: 12.0 MiB / 64.0 MiB (19%)"   (percent rounded to an integer)
//!     ///   unlimited:  "meshes: 3.5 MiB"
//!     ///   over limit: "shadows: 20.0 MiB / 16.0 MiB (125%) OVER"
//!     ///   total:      "total: 35.5 MiB"
//!     /// A limit of 0 prints no percent: "buffers: 1.0 MiB / 0.0 MiB OVER".
//!     pub fn report(&self) -> String;
//! }
//! ```

#![allow(clippy::unwrap_used)]

use redox_render::{MemoryBudget, MemoryCategory, texture_size_bytes};

const MIB: u64 = 1024 * 1024;

#[test]
fn a_single_level_texture_is_width_times_height_times_pixel_size() {
    assert_eq!(texture_size_bytes(256, 128, 4, 1), 256 * 128 * 4);
    assert_eq!(
        texture_size_bytes(256, 128, 4, 0),
        256 * 128 * 4,
        "0 levels counts as 1"
    );
}

#[test]
fn mip_levels_are_summed_down_to_one_pixel() {
    // 4x4 RGBA8 with 3 levels: 4x4 + 2x2 + 1x1 = 21 pixels.
    assert_eq!(texture_size_bytes(4, 4, 4, 3), 21 * 4);
    // More levels than the image has: the 1x1 level repeats.
    assert_eq!(texture_size_bytes(2, 2, 1, 5), 4 + 1 + 1 + 1 + 1);
    // Non-square.
    assert_eq!(texture_size_bytes(8, 2, 1, 4), 16 + 4 + 2 + 1);
}

#[test]
fn huge_textures_saturate_instead_of_overflowing() {
    assert_eq!(
        texture_size_bytes(u32::MAX, u32::MAX, u32::MAX, 32),
        u64::MAX
    );
}

#[test]
fn over_budget_categories_lists_only_the_overloaded_ones_in_order() {
    let mut b = MemoryBudget::new();
    b.set_limit(MemoryCategory::Textures, Some(10));
    b.set_limit(MemoryCategory::Shadows, Some(10));
    b.set_limit(MemoryCategory::Buffers, Some(10));
    b.add(MemoryCategory::Textures, 11);
    b.add(MemoryCategory::Shadows, 10);
    b.add(MemoryCategory::Buffers, 99);
    b.add(MemoryCategory::Meshes, 1_000_000); // no limit: never over
    assert_eq!(
        b.over_budget_categories(),
        vec![MemoryCategory::Textures, MemoryCategory::Buffers]
    );
}

#[test]
fn an_empty_budget_reports_only_the_total() {
    assert_eq!(MemoryBudget::new().report(), "total: 0.0 MiB");
}

#[test]
fn the_report_formats_limited_unlimited_and_over_budget_lines() {
    let mut b = MemoryBudget::new();
    b.set_limit(MemoryCategory::Textures, Some(64 * MIB));
    b.add(MemoryCategory::Textures, 12 * MIB);
    b.add(MemoryCategory::Meshes, 3 * MIB + MIB / 2);
    b.set_limit(MemoryCategory::Shadows, Some(16 * MIB));
    b.add(MemoryCategory::Shadows, 20 * MIB);
    let expected = "textures: 12.0 MiB / 64.0 MiB (19%)\n\
                    meshes: 3.5 MiB\n\
                    shadows: 20.0 MiB / 16.0 MiB (125%) OVER\n\
                    total: 35.5 MiB";
    assert_eq!(b.report(), expected);
}

#[test]
fn a_zero_limit_prints_no_percent() {
    let mut b = MemoryBudget::new();
    b.set_limit(MemoryCategory::Buffers, Some(0));
    b.add(MemoryCategory::Buffers, MIB);
    assert_eq!(
        b.report(),
        "buffers: 1.0 MiB / 0.0 MiB OVER\ntotal: 1.0 MiB"
    );
}

#[test]
fn a_limit_without_usage_is_still_listed() {
    let mut b = MemoryBudget::new();
    b.set_limit(MemoryCategory::VirtualGeometry, Some(8 * MIB));
    assert_eq!(
        b.report(),
        "virtual_geometry: 0.0 MiB / 8.0 MiB (0%)\ntotal: 0.0 MiB"
    );
}

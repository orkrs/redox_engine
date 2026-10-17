//! Specification (tests first) for the GPU memory budget accounting (plan item A4, first part).
//!
//! A pure bookkeeping structure, no GPU involved. API to implement in a NEW file
//! `crates/redox_render/src/memory_budget.rs`, declared and re-exported in `lib.rs`
//! (`pub mod memory_budget;` and `pub use memory_budget::{CategoryUsage, MemoryBudget, MemoryCategory};`).
//! Do not wire it into the renderer (that is a separate step).
//!
//! ```text
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
//! pub enum MemoryCategory { Textures, Meshes, Shadows, VirtualGeometry, Buffers, Other }
//! impl MemoryCategory { pub const ALL: [MemoryCategory; 6]; pub fn name(self) -> &'static str }  // "textures", ...
//! #[derive(Clone, Copy, Debug, PartialEq)]
//! pub struct CategoryUsage { pub category: MemoryCategory, pub used: u64, pub limit: Option<u64>,
//!                            pub fraction: Option<f32> }   // used / limit, None without a limit (or limit 0)
//! #[derive(Clone, Debug, Default)]
//! pub struct MemoryBudget;
//! impl MemoryBudget {
//!     pub fn new() -> Self;                                  // everything 0, no limits
//!     pub fn set_limit(&mut self, category: MemoryCategory, bytes: Option<u64>);
//!     pub fn limit(&self, category: MemoryCategory) -> Option<u64>;
//!     pub fn used(&self, category: MemoryCategory) -> u64;
//!     pub fn total_used(&self) -> u64;
//!     /// Adds bytes unconditionally (saturating). Real resources exist even when over budget.
//!     pub fn add(&mut self, category: MemoryCategory, bytes: u64);
//!     /// Removes bytes, saturating at 0.
//!     pub fn sub(&mut self, category: MemoryCategory, bytes: u64);
//!     /// Adds only when the category stays within its limit (no limit = always); returns whether it did.
//!     pub fn try_reserve(&mut self, category: MemoryCategory, bytes: u64) -> bool;
//!     /// Bytes still available before the limit; `None` without a limit; 0 when over.
//!     pub fn remaining(&self, category: MemoryCategory) -> Option<u64>;
//!     pub fn is_over_budget(&self, category: MemoryCategory) -> bool;   // used > limit
//!     /// One entry per category in `MemoryCategory::ALL` order.
//!     pub fn snapshot(&self) -> Vec<CategoryUsage>;
//! }
//! ```

#![allow(clippy::unwrap_used)]

use redox_render::memory_budget::{CategoryUsage, MemoryBudget, MemoryCategory};

use MemoryCategory::*;

#[test]
fn a_new_budget_is_empty_and_unlimited() {
    let b = MemoryBudget::new();
    assert_eq!(b.total_used(), 0);
    for c in MemoryCategory::ALL {
        assert_eq!(b.used(c), 0);
        assert_eq!(b.limit(c), None);
        assert_eq!(b.remaining(c), None);
        assert!(!b.is_over_budget(c));
    }
}

#[test]
fn add_and_sub_track_each_category_separately() {
    let mut b = MemoryBudget::new();
    b.add(Textures, 1000);
    b.add(Meshes, 500);
    b.add(Textures, 24);
    assert_eq!(b.used(Textures), 1024);
    assert_eq!(b.used(Meshes), 500);
    assert_eq!(b.total_used(), 1524);
    b.sub(Textures, 24);
    assert_eq!(b.used(Textures), 1000);
}

#[test]
fn sub_and_add_saturate() {
    let mut b = MemoryBudget::new();
    b.add(Shadows, 10);
    b.sub(Shadows, 999);
    assert_eq!(b.used(Shadows), 0);
    b.add(Other, u64::MAX);
    b.add(Other, 5);
    assert_eq!(b.used(Other), u64::MAX);
}

#[test]
fn limits_drive_remaining_and_over_budget() {
    let mut b = MemoryBudget::new();
    b.set_limit(Textures, Some(1000));
    b.add(Textures, 400);
    assert_eq!(b.remaining(Textures), Some(600));
    assert!(!b.is_over_budget(Textures));
    b.add(Textures, 700); // add never refuses
    assert_eq!(b.used(Textures), 1100);
    assert_eq!(b.remaining(Textures), Some(0));
    assert!(b.is_over_budget(Textures));
    b.set_limit(Textures, None);
    assert!(!b.is_over_budget(Textures));
    assert_eq!(b.limit(Textures), None);
}

#[test]
fn try_reserve_respects_the_limit() {
    let mut b = MemoryBudget::new();
    b.set_limit(Shadows, Some(100));
    assert!(b.try_reserve(Shadows, 60));
    assert!(!b.try_reserve(Shadows, 41), "would exceed by 1");
    assert_eq!(b.used(Shadows), 60, "a refusal changes nothing");
    assert!(b.try_reserve(Shadows, 40), "exactly at the limit is fine");
    assert!(b.try_reserve(Meshes, u64::MAX), "no limit: always");
}

#[test]
fn a_zero_limit_refuses_everything() {
    let mut b = MemoryBudget::new();
    b.set_limit(Buffers, Some(0));
    assert!(!b.try_reserve(Buffers, 1));
    assert!(b.try_reserve(Buffers, 0));
}

#[test]
fn the_snapshot_lists_every_category_in_order() {
    let mut b = MemoryBudget::new();
    b.set_limit(Textures, Some(2000));
    b.add(Textures, 500);
    b.add(VirtualGeometry, 7);
    let s = b.snapshot();
    assert_eq!(s.len(), MemoryCategory::ALL.len());
    let order: Vec<_> = s.iter().map(|u| u.category).collect();
    assert_eq!(order, MemoryCategory::ALL.to_vec());
    assert_eq!(
        s[0],
        CategoryUsage {
            category: Textures,
            used: 500,
            limit: Some(2000),
            fraction: Some(0.25)
        }
    );
    let vg = s.iter().find(|u| u.category == VirtualGeometry).unwrap();
    assert_eq!((vg.used, vg.limit, vg.fraction), (7, None, None));
}

#[test]
fn a_zero_limit_has_no_fraction() {
    let mut b = MemoryBudget::new();
    b.set_limit(Other, Some(0));
    b.add(Other, 5);
    assert_eq!(b.snapshot().last().unwrap().fraction, None);
}

#[test]
fn category_names_are_stable_and_distinct() {
    let names: Vec<_> = MemoryCategory::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(names[0], "textures");
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len());
}

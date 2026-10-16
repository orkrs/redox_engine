use super::super::page_table::PAGE_CACHED_BIT;
use super::*;

fn clipmap() -> VsmClipmap {
    VsmClipmap::new(VsmConfig {
        level_count: 4,
        ..VsmConfig::default()
    })
}

/// Bitfield with exactly one requested flat page index.
fn request_bits(flat_page: usize) -> Vec<u32> {
    let mut bits = vec![0u32; VSM_PAGE_TABLE_WORDS.div_ceil(32)];
    bits[flat_page / 32] |= 1 << (flat_page % 32);
    bits
}

#[test]
fn update_builds_levels_and_allocates_on_request() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    assert_eq!(cm.levels().len(), 4);
    assert_eq!(cm.tables().len(), 4);

    // Request page (0, 0) of level 0.
    let bits = request_bits(0);
    let dirty = cm.requested_dirty_pages(&bits);
    assert_eq!(dirty.len(), 1);
    assert_eq!((dirty[0].0, dirty[0].1, dirty[0].2), (0, 0, 0));
    assert!(dirty[0].3 < cm.config().pool_pages as u16);
    assert_eq!(cm.stats().pages_allocated, 1);
    assert_eq!(cm.stats().pages_dirty, 1);
    cm.mark_pages_cached(&dirty);

    // Second call with no invalidation: page now cached, nothing dirty.
    assert!(cm.requested_dirty_pages(&bits).is_empty());
    assert_eq!(cm.stats().pages_overflow, 0);
}

#[test]
fn overflow_requests_are_dropped_safely() {
    let mut cm = VsmClipmap::new(VsmConfig {
        level_count: 2,
        pool_pages: 1,
        ..VsmConfig::default()
    });
    cm.update(Vec3::ZERO, Vec3::Y);
    let mut bits = request_bits(0);
    let flat1 = 256;
    bits[flat1 / 32] |= 1 << (flat1 % 32);
    let dirty = cm.requested_dirty_pages(&bits);
    assert_eq!(dirty.len(), 1);
    assert!(cm.stats().pages_overflow <= 1);
}

#[test]
fn scrolling_shifts_entries_without_leaks() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    // With 1 page per side, single page covers whole window — sliding by one page drops it.
    let phys = cm.phys_pool_mut().alloc().unwrap();
    cm.tables_mut()[0].set(0, 0, phys as u32 | PAGE_VALID_BIT | PAGE_CACHED_BIT);

    let page_world =
        (VSM_PAGES_PER_SIDE as f32 * cm.config().page_px as f32) * cm.levels()[0].texel_world;
    cm.update(Vec3::Z * page_world, Vec3::Y);

    let pt = &cm.tables()[0];
    // Single page has no neighbor to shift into — old entry is dropped, table cleared.
    assert_eq!(pt.get(0, 0), 0, "single page window slid out");
    assert_eq!(
        cm.phys_pool_mut().free_count(),
        cm.phys_pool_mut().capacity()
    );
}

#[test]
fn scrolled_out_entries_return_pages_to_pool() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    let phys = cm.phys_pool_mut().alloc().unwrap();
    cm.tables_mut()[0].set(0, 0, phys as u32 | PAGE_VALID_BIT | PAGE_CACHED_BIT);

    // Slide the window 1 page: single page falls out.
    let page_world =
        (VSM_PAGES_PER_SIDE as f32 * cm.config().page_px as f32) * cm.levels()[0].texel_world;
    cm.update(Vec3::Z * page_world, Vec3::Y);

    assert_eq!(cm.stats().pages_freed, 1);
    assert_eq!(cm.tables()[0].get(0, 0), 0);
    assert_eq!(
        cm.phys_pool_mut().free_count(),
        cm.phys_pool_mut().capacity()
    );
}

fn single_level(pool_pages: u32) -> VsmClipmap {
    let mut cm = VsmClipmap::new(VsmConfig {
        level_count: 1,
        pool_pages,
        ..VsmConfig::default()
    });
    cm.update(Vec3::ZERO, Vec3::Y);
    cm
}

#[test]
fn full_pool_evicts_the_least_recently_requested_page() {
    let mut cm = single_level(2);
    cm.requested_dirty_pages(&request_bits(0));
    cm.requested_dirty_pages(&request_bits(1));
    // Page 2 needs a slot: page 0 was requested longest ago and must be the victim.
    cm.requested_dirty_pages(&request_bits(2));
    assert_eq!(cm.stats().pages_overflow, 0);
    assert_eq!(cm.tables()[0].entries[0], 0, "oldest page evicted");
    assert_ne!(cm.tables()[0].entries[1], 0, "newer page kept");
    assert_ne!(cm.tables()[0].entries[2], 0, "new page resident");
}

#[test]
fn pages_requested_this_frame_are_never_evicted() {
    let mut cm = single_level(1);
    let mut both = request_bits(0);
    both[0] |= 1 << 1;
    cm.requested_dirty_pages(&both);
    assert_eq!(
        cm.stats().pages_overflow,
        1,
        "second page must overflow, not steal the first"
    );
    assert_ne!(cm.tables()[0].entries[0], 0);
}

#[test]
fn unrequested_pages_are_released_after_the_stale_window() {
    let mut cm = single_level(4);
    cm.requested_dirty_pages(&request_bits(0));
    assert_ne!(cm.tables()[0].entries[0], 0);
    for _ in 0..=STALE_PAGE_FRAMES {
        cm.requested_dirty_pages(&[]);
    }
    assert_eq!(
        cm.tables()[0].entries[0],
        0,
        "stale page returned to the pool"
    );
    assert_eq!(
        cm.phys_pool_mut().free_count(),
        cm.phys_pool_mut().capacity()
    );
}

/// Page `(px, py)` of `level` that covers `world`, using the same mapping as the shaders.
fn page_covering(cm: &VsmClipmap, level: usize, world: Vec3) -> Option<(u32, u32)> {
    let clip = cm.levels()[level].view_proj * Vec4::new(world.x, world.y, world.z, 1.0);
    let ndc = Vec3::new(clip.x, clip.y, clip.z) / clip.w;
    let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if !(0.0..1.0).contains(&uv.x) || !(0.0..1.0).contains(&uv.y) {
        return None;
    }
    let pps = VSM_PAGES_PER_SIDE as f32;
    Some(((uv.x * pps) as u32, (uv.y * pps) as u32))
}

/// A cached page must keep covering the same world point when the window scrolls, along
/// either axis and in either direction (the y axis is flipped in page space).
#[test]
fn cached_pages_follow_their_world_region_when_the_window_scrolls() {
    let point = Vec3::new(1.3, 0.0, -0.9);
    let step = |cm: &VsmClipmap| cm.config().page_px as f32 * cm.levels()[0].texel_world;
    for (move_x, move_z) in [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (2.0, -3.0),
    ] {
        let mut cm = single_level(4);
        let (px, py) = page_covering(&cm, 0, point).expect("point inside the window");
        let phys = cm.phys_pool_mut().alloc().unwrap();
        cm.tables_mut()[0].set(px, py, phys as u32 | PAGE_VALID_BIT | PAGE_CACHED_BIT);

        let page = step(&cm);
        cm.update(Vec3::new(move_x * page, 0.0, move_z * page), Vec3::Y);

        let (nx, ny) = page_covering(&cm, 0, point).expect("point still inside the window");
        assert_ne!(
            cm.tables()[0].get(nx, ny) & PAGE_VALID_BIT,
            0,
            "move ({move_x}, {move_z}): page for the point was {:?}, now {:?} but holds no data",
            (px, py),
            (nx, ny)
        );
    }
}

fn cached_page_covering(cm: &mut VsmClipmap, world: Vec3) -> (u32, u32) {
    let (px, py) = page_covering(cm, 0, world).expect("point inside the window");
    let phys = cm.phys_pool_mut().alloc().unwrap();
    cm.tables_mut()[0].set(px, py, phys as u32 | PAGE_VALID_BIT | PAGE_CACHED_BIT);
    (px, py)
}

#[test]
fn invalidated_pages_are_rendered_in_the_same_frame_without_feedback() {
    let mut cm = single_level(8);
    let point = Vec3::new(0.5, 0.0, 0.5);
    let (px, py) = cached_page_covering(&mut cm, point);
    let aabb = redox_math::Aabb {
        min: point - Vec3::splat(0.05),
        max: point + Vec3::splat(0.05),
    };

    cm.invalidate_aabbs(&[aabb]);
    // No screen feedback at all: the forced request alone must re-render the page.
    let dirty = cm.requested_dirty_pages(&[]);
    assert!(
        dirty.iter().any(|&(_, x, y, _)| (x, y) == (px, py)),
        "dirty: {dirty:?}"
    );
    // And it is consumed: the next frame does not re-render it again.
    cm.mark_pages_cached(&dirty);
    assert!(cm.requested_dirty_pages(&[]).is_empty());
}

#[test]
fn invalidation_uses_all_box_corners_even_beyond_the_window() {
    let mut cm = single_level(8);
    let point = Vec3::new(0.5, 0.0, 0.5);
    let (px, py) = cached_page_covering(&mut cm, point);
    // A huge box that covers the window and extends far past its edge in every direction.
    let aabb = redox_math::Aabb {
        min: Vec3::splat(-500.0),
        max: Vec3::splat(500.0),
    };
    cm.invalidate_aabbs(&[aabb]);
    assert_eq!(
        cm.tables()[0].get(px, py) & PAGE_CACHED_BIT,
        0,
        "a box larger than the window must still invalidate the pages inside it"
    );
}

#[test]
fn coarse_fallback_keeps_only_the_coarsest_levels() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    let aabb = redox_math::Aabb {
        min: Vec3::splat(-1.0),
        max: Vec3::splat(1.0),
    };
    let all = cm.request_bits_covering_aabbs(&[aabb]);
    let coarse = cm.coarse_fallback_bits(&[aabb], 1);
    let pages_per_level = (VSM_PAGES_PER_SIDE * VSM_PAGES_PER_SIDE) as usize;
    let count = |bits: &[u32], levels: std::ops::Range<usize>| -> u32 {
        (levels.start * pages_per_level..levels.end * pages_per_level)
            .filter(|f| bits[f / 32] & (1 << (f % 32)) != 0)
            .count() as u32
    };
    assert!(
        count(&all, 0..3) > 0,
        "the full request reaches fine levels"
    );
    assert_eq!(count(&coarse, 0..3), 0, "fallback drops the fine levels");
    assert!(count(&coarse, 3..4) > 0, "and keeps the coarsest");
}

#[test]
fn depth_plane_is_snapped_and_a_step_drops_cached_pages() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    let phys = cm.phys_pool_mut().alloc().unwrap();
    cm.tables_mut()[0].set(0, 0, phys as u32 | PAGE_VALID_BIT | PAGE_CACHED_BIT);
    let half = cm.levels()[0].world_half_extent;

    // Drift well below a depth step: the plane does not move, the cached page survives.
    cm.update(Vec3::Y * (0.1 * half), Vec3::Y);
    assert_ne!(
        cm.tables()[0].get(0, 0),
        0,
        "small drift must keep cached pages"
    );
    assert_eq!(cm.levels()[0].center_light().z, 0.0);

    // Moving several steps along the light direction drops the level's stale pages.
    cm.update(Vec3::Y * (3.0 * half), Vec3::Y);
    assert_eq!(
        cm.tables()[0].get(0, 0),
        0,
        "stale depth must not survive a plane step"
    );
    assert_eq!(cm.stats().pages_freed, 1);
    assert_eq!(
        cm.phys_pool_mut().free_count(),
        cm.phys_pool_mut().capacity()
    );
    let z = cm.levels()[0].center_light().z;
    assert!(
        (z / (half * 0.5)).fract().abs() < 1.0e-3,
        "plane z {z} is on the snap grid"
    );
}

#[test]
fn light_moved_frees_all_pages() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    let dirty = cm.requested_dirty_pages(&request_bits(0));
    cm.mark_pages_cached(&dirty);
    assert_eq!(cm.stats().pages_allocated, 1);

    cm.update(Vec3::ONE, Vec3::Z); // different direction → LightMoved
    assert_eq!(cm.stats().pages_freed, 1);
    assert!(
        cm.tables()
            .iter()
            .all(|t| t.entries.iter().all(|&e| e == 0))
    );
    assert_eq!(
        cm.phys_pool_mut().free_count(),
        cm.phys_pool_mut().capacity()
    );
}

#[test]
fn dynamic_object_invalidation_keeps_pages_but_dirties_them() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    let dirty = cm.requested_dirty_pages(&request_bits(0));
    cm.mark_pages_cached(&dirty);
    cm.invalidate(&[InvalidationEvent::DynamicObject {
        level: 0,
        page_min_x: 0,
        page_min_y: 0,
        page_max_x: 0,
        page_max_y: 0,
    }]);
    let entry = cm.tables()[0].get(0, 0);
    assert_ne!(entry & PAGE_VALID_BIT, 0, "page stays allocated");
    assert_eq!(entry & PAGE_CACHED_BIT, 0, "but becomes dirty");
    assert_eq!(cm.stats().pages_freed, 0);
    assert_eq!(cm.requested_dirty_pages(&request_bits(0)).len(), 1);
}

#[test]
fn gpu_info_layout_matches_wgsl_contract() {
    // (WGSL: max_trace + vec3 pad → offset 64).
    let header_bytes = 64;
    let expected = header_bytes + 8 * std::mem::size_of::<VsmLevelGpu>() + VSM_PAGE_TABLE_WORDS * 4;
    assert_eq!(std::mem::size_of::<VsmGpuInfo>(), expected);
    assert_eq!(expected, 8896, "must match WGSL VsmGpuInfo size");
    assert_eq!(header_bytes % 16, 0, "levels array must be 16-aligned");

    let mut cm = clipmap();
    cm.update(Vec3::new(3.0, 1.0, -2.0), Vec3::Y);
    let info = cm.gpu_info(&SmrtConfig::default());
    assert_eq!(info.level_count, 4);
    assert_eq!(info.ray_count, 4);
    assert_eq!(info.samples_per_ray, 5);
    assert_eq!(info.page_tables.len(), VSM_PAGE_TABLE_WORDS);
    assert_eq!(info.page_px, VsmConfig::default().page_px);
    assert_eq!(info.levels[0].page_table_offset, 0);
    assert_eq!(info.levels[1].page_table_offset, 256);
}

#[test]
fn page_view_proj_maps_page_edges_to_ndc() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    let level_vp = cm.levels()[0].view_proj;
    let pps = VSM_PAGES_PER_SIDE as f32;
    let page_px = cm.config().page_px as f32;
    let page_x = 0u32;
    let page_y = 0u32;
    let page_vp = cm.page_view_proj(0, page_x, page_y);
    // page_vp * inv(level_vp) is the NDC scale-bias; apply to page corners.
    let m = page_vp * level_vp.inverse();
    let map_ndc = |nx: f32, ny: f32| {
        let c0 = m.col(0);
        let c1 = m.col(1);
        let c3 = m.col(3);
        let x = c0.x * nx + c1.x * ny + c3.x;
        let y = c0.y * nx + c1.y * ny + c3.y;
        let w = c0.w * nx + c1.w * ny + c3.w;
        (x / w, y / w)
    };

    // Flipped UV.y: uv=(px/pps, py/pps) is NDC top of the page → clip +Y.
    let nx0 = 2.0 * page_x as f32 / pps - 1.0;
    let nx1 = 2.0 * (page_x as f32 + 1.0) / pps - 1.0;
    let ny0 = 1.0 - 2.0 * page_y as f32 / pps;
    let ny1 = 1.0 - 2.0 * (page_y as f32 + 1.0) / pps;

    // With 1-texel page overlap, logical page edges land at ±page_px/(page_px+2).
    let edge = page_px / (page_px + 2.0);
    let (lx, ly) = map_ndc(nx0, ny0);
    let (rx, ry) = map_ndc(nx1, ny1);
    assert!(
        (lx + edge).abs() < 1e-3 && (ly - edge).abs() < 1e-3,
        "page UV min → clip (-edge,+edge), got ({lx}, {ly}) edge={edge}"
    );
    assert!(
        (rx - edge).abs() < 1e-3 && (ry + edge).abs() < 1e-3,
        "page UV max → clip (+edge,-edge), got ({rx}, {ry}) edge={edge}"
    );
}

#[test]
fn request_bits_covering_origin_requests_a_page() {
    let mut cm = clipmap();
    cm.update(Vec3::ZERO, Vec3::Y);
    let aabb = redox_math::Aabb {
        min: Vec3::splat(-1.0),
        max: Vec3::splat(1.0),
    };
    let bits = cm.request_bits_covering_aabbs(&[aabb]);
    assert!(
        bits.iter().any(|&w| w != 0),
        "origin must land in some clipmap page"
    );
    let dirty = cm.requested_dirty_pages(&bits);
    assert!(!dirty.is_empty());
}

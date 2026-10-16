//! Directional-light VSM clipmap: per-level light-space windows around the
//! camera, page-table scrolling, and dirty-page bookkeeping.
//!
//! The clipmap owns, per level:
//! * an orthographic view-projection snapped to the level's page grid
//!   (snapping keeps shadow texels world-stable while the camera moves),
//! * a [`VirtualPageTable`] translating that grid into physical atlas pages,
//!
//! plus one shared [`PhysPageAllocator`] for the whole pool.
//!
//! Frame flow (VSM mode):
//! 1. `update(camera_pos, light_dir)` — rebuild matrices, scroll tables,
//!    invalidate everything on `LightMoved`.
//! 2. GPU visibility pass (`shadow/visibility.rs`) marks requested pages as
//!    bits in a storage buffer.
//! 3. `requested_dirty_pages(bits)` allocates physical pages for newly
//!    visible entries and returns the dirty set via
//!    [`DirtyPageCollector::collect`](super::cache::DirtyPageCollector::collect).
//! 4. The depth pass renders only those pages (`shadow/depth_pass.rs`).
//!
//! This module is CPU-only state; GPU upload happens through
//! [`VsmClipmap::gpu_info`] producing a bytemuck-flat [`VsmGpuInfo`].

use bytemuck::Zeroable;
use redox_math::{Aabb, Mat4, Vec2, Vec3, Vec4, look_at, orthographic};

use super::cache::{DirtyPageCollector, InvalidationEvent, process_invalidations};
use super::page_table::{
    PAGE_CACHED_BIT, PAGE_VALID_BIT, PhysPageAllocator, VirtualPageTable, phys_page,
};
use super::smrt::SmrtConfig;

/// Pages along one axis of every clipmap level's virtual window.
pub const VSM_PAGES_PER_SIDE: u32 = 16;
pub const VSM_MAX_LEVELS: usize = 8;
/// Full world-space width of level 0; each coarser level doubles it. With 256 px pages and 16
/// pages per side level 0 has 0.4 cm texels; five levels reach 256 m (6 cm texels).
const BASE_WORLD_SIZE: f32 = 16.0;
/// Frames a page may go unrequested before it is returned to the pool.
const STALE_PAGE_FRAMES: u64 = 240;
/// Light-direction change (radians) large enough to count as "the light
/// moved", triggering full invalidation.
const LIGHT_MOVED_EPSILON: f32 = 1e-4;

/// One dirty (valid, uncached, requested) page awaiting re-render.
pub type DirtyPage = (u32, u32, u32, u16);

/// CPU-side VSM clipmap configuration.
#[derive(Clone, Debug)]
pub struct VsmConfig {
    /// Number of clipmap levels (each covers twice the extent of the previous).
    pub level_count: usize,
    /// Page size in texels.
    pub page_px: u32,
    /// Physical pages in the shared atlas pool.
    pub pool_pages: u32,
}

impl Default for VsmConfig {
    fn default() -> Self {
        Self {
            level_count: 5,
            // 256 px pages at the same page count: 4x the texels per level (level 0 texel =
            // 64 m / (16 * 256) = 1.6 cm). The pool is 256 layers of R32Float = 64 MB.
            page_px: 256,
            pool_pages: 256,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ClipmapLevel {
    /// World→light-space projection covering this level's window.
    pub view_proj: Mat4,
    /// Half of the world-space width covered by this level.
    pub world_half_extent: f32,
    /// World units per shadow texel at this level.
    pub texel_world: f32,
    /// Snapped light-plane center `(sx, sy)` plus depth offset `sz`.
    center_light: Vec3,
}

impl ClipmapLevel {
    /// Snapped light-plane center of the window.
    pub fn center_light(&self) -> Vec3 {
        self.center_light
    }
}

/// Counters surfaced for HUDs (plan P2: dirty-page redraw counter).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VsmStats {
    /// Physical pages allocated since creation.
    pub pages_allocated: u64,
    /// Physical pages returned to the pool since creation.
    pub pages_freed: u64,
    /// Pages scheduled for rendering last frame.
    pub pages_dirty: usize,
    /// Requests dropped because the pool was exhausted.
    pub pages_overflow: u64,
}

/// Directional-light VSM clipmap state (see [module docs](self)).
pub struct VsmClipmap {
    config: VsmConfig,
    levels: Vec<ClipmapLevel>,
    tables: Vec<VirtualPageTable>,
    phys_pool: PhysPageAllocator,
    stats: VsmStats,
    prev_light_dir: Option<Vec3>,
    /// Incremented by every [`Self::requested_dirty_pages`] call.
    frame: u64,
    /// Per level and page: the last frame the page was requested (LRU eviction).
    last_used: Vec<Vec<u64>>,
    /// Pages invalidated by moving casters; merged into the next request so they re-render
    /// immediately.
    forced: Vec<u32>,
}

impl VsmClipmap {
    /// Creates the clipmap; `level_count` is clamped to [`VSM_MAX_LEVELS`].
    pub fn new(config: VsmConfig) -> Self {
        let level_count = config.level_count.clamp(1, VSM_MAX_LEVELS);
        let mut config = config;
        config.level_count = level_count;
        let levels = (0..level_count)
            .map(|i| {
                let world_size = BASE_WORLD_SIZE * (1u32 << i) as f32;
                ClipmapLevel {
                    view_proj: Mat4::IDENTITY,
                    world_half_extent: world_size * 0.5,
                    texel_world: world_size / (VSM_PAGES_PER_SIDE * config.page_px) as f32,
                    center_light: Vec3::ZERO,
                }
            })
            .collect();
        Self {
            levels,
            tables: (0..level_count)
                .map(|_| VirtualPageTable::new(VSM_PAGES_PER_SIDE, VSM_PAGES_PER_SIDE))
                .collect(),
            phys_pool: PhysPageAllocator::new(config.pool_pages),
            config,
            stats: VsmStats::default(),
            prev_light_dir: None,
            frame: 0,
            last_used: (0..level_count)
                .map(|_| vec![0; (VSM_PAGES_PER_SIDE * VSM_PAGES_PER_SIDE) as usize])
                .collect(),
            forced: vec![
                0;
                (level_count * (VSM_PAGES_PER_SIDE * VSM_PAGES_PER_SIDE) as usize)
                    .div_ceil(32)
            ],
        }
    }

    pub fn config(&self) -> &VsmConfig {
        &self.config
    }

    pub fn levels(&self) -> &[ClipmapLevel] {
        &self.levels
    }

    pub fn tables(&self) -> &[VirtualPageTable] {
        &self.tables
    }

    pub fn tables_mut(&mut self) -> &mut [VirtualPageTable] {
        &mut self.tables
    }

    /// `(free, capacity)` of the physical page pool.
    pub fn pool_usage(&self) -> (usize, usize) {
        (self.phys_pool.free_count(), self.phys_pool.capacity())
    }

    pub fn phys_pool_mut(&mut self) -> &mut PhysPageAllocator {
        &mut self.phys_pool
    }

    pub fn stats(&self) -> VsmStats {
        self.stats
    }

    /// Per-frame update: rebuilds light-space windows around `camera_pos`
    /// for a directional light shining **from** `-light_dir` (i.e.
    /// `light_dir` points toward the light, matching the shading convention
    /// in `pbr.wgsl`). Invalidates all pages when the direction changed.
    pub fn update(&mut self, camera_pos: Vec3, light_dir: Vec3) {
        let dir = if light_dir.length_squared() > 1e-8 {
            light_dir.normalize()
        } else {
            Vec3::Y
        };

        if self
            .prev_light_dir
            .is_some_and(|prev| prev.angle_between(dir) > LIGHT_MOVED_EPSILON)
        {
            self.invalidate(&[InvalidationEvent::LightMoved]);
        }
        self.prev_light_dir = Some(dir);

        // Orthonormal light basis: z points toward the light, x/y span the
        // shadowed plane.
        let up_world = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
        let z = dir;
        let x_axis = up_world.cross(z).normalize();
        let y_axis = z.cross(x_axis);

        for i in 0..self.levels.len() {
            let (half, texel_world, prev) = {
                let level = &self.levels[i];
                (
                    level.world_half_extent,
                    level.texel_world,
                    level.center_light,
                )
            };
            // Depth range spans ±(2·half) around the center plane so casters
            // above/below stay inside; ndc-z→world scale is (far − near).
            let near = -2.0 * half;
            let far = 2.0 * half;
            // One page in world units: the window follows the camera in page steps (not in
            // whole-window jumps), so fine levels stay centred on the viewer.
            let page_world = self.config.page_px as f32 * texel_world;

            // Camera position expressed in the light basis, xy snapped to
            // page boundaries so shadow texels never swim.
            let cx = camera_pos.dot(x_axis);
            let cy = camera_pos.dot(y_axis);
            let cz = camera_pos.dot(z);
            let sx = (cx / page_world).round() * page_world;
            let sy = (cy / page_world).round() * page_world;

            // Scroll by whole pages when the window moved. On the very first
            // update every table is still empty, so shifting is a harmless
            // no-op and needs no special case.
            // Page rows run against the light-plane y axis (uv.y = 0.5 - ndc.y * 0.5), so a
            // positive window move along y moves page content to *higher* rows: the table
            // shift (which moves content by -d) takes the negated step. x runs with the axis.
            let dx = ((sx - prev.x) / page_world).round() as i32;
            let dy = -(((sy - prev.y) / page_world).round() as i32);
            if dx != 0 || dy != 0 {
                for phys in shift_entries(&mut self.tables[i], dx, dy) {
                    self.phys_pool.free(phys);
                    self.stats.pages_freed += 1;
                }
                shift_grid(&mut self.last_used[i], VSM_PAGES_PER_SIDE as i32, dx, dy);
                // Keep overlapping pages CACHED — only newly exposed edge
                // pages (currently invalid) will be allocated as dirty.
            }

            // Page depth is stored relative to this level's plane along the light direction.
            // Snap that plane like xy: with a continuously moving plane every cached page
            // keeps depth for an old plane, so receivers compare against stale values and
            // whole page-sized regions turn dark. When the plane does step, all pages of the
            // level hold depth for the old plane and are dropped (re-rendered on demand).
            // The step is a quarter of the depth range (+-2h), so the camera stays within
            // 0.25h of the plane and casters up to 1.75h away along the light still fit.
            let depth_step = (half * 0.5).max(f32::EPSILON);
            let sz = (cz / depth_step).round() * depth_step;
            if (sz - prev.z).abs() > depth_step * 0.5 {
                self.release_level_pages(i);
            }

            let eye = x_axis * sx + y_axis * sy + z * sz;
            let view = look_at(eye, eye - z, y_axis);
            let level = &mut self.levels[i];
            level.view_proj = orthographic(-half, half, -half, half, near, far) * view;
            level.center_light = Vec3::new(sx, sy, sz);
        }
    }

    /// Applies invalidation events (see [`process_invalidations`]).
    ///
    /// Unlike raw [`VirtualPageTable::invalidate_all`], full invalidations
    /// first return every allocated physical page to the pool.
    pub fn invalidate(&mut self, events: &[InvalidationEvent]) {
        let needs_full_release = events
            .iter()
            .any(|e| matches!(e, InvalidationEvent::Full | InvalidationEvent::LightMoved));
        if needs_full_release {
            self.release_all_pages();
        }
        process_invalidations(events, &mut self.tables);
    }

    /// Returns the physical pages of one level to the pool and clears its table.
    fn release_level_pages(&mut self, level: usize) {
        let pt = &mut self.tables[level];
        for &entry in &pt.entries {
            if entry & PAGE_VALID_BIT != 0 {
                self.phys_pool.free(phys_page(entry));
                self.stats.pages_freed += 1;
            }
        }
        pt.invalidate_all();
    }

    /// Returns every physical page to the pool and clears all tables.
    pub fn release_all_pages(&mut self) {
        for pt in &mut self.tables {
            for &entry in &pt.entries {
                if entry & PAGE_VALID_BIT != 0 {
                    self.phys_pool.free(phys_page(entry));
                    self.stats.pages_freed += 1;
                }
            }
            pt.invalidate_all();
        }
    }

    /// Marks pages covering `aabbs` as dirty (clears `CACHED`) so moving
    /// objects re-render. Called before `requested_dirty_pages` each frame.
    /// Cost is O(levels * aabbs * pages), but aabbs are few (crystals/sentinels)
    /// and dirty re-render is limited to overlapping pages.
    pub fn invalidate_aabbs(&mut self, aabbs: &[redox_math::Aabb]) {
        if aabbs.is_empty() {
            return;
        }
        let pps = VSM_PAGES_PER_SIDE;
        let pages_per_level = (pps * pps) as usize;
        for (lvl_idx, level) in self.levels.iter().enumerate() {
            let pt = &mut self.tables[lvl_idx];
            for aabb in aabbs {
                // Light-plane footprint of the box from ALL corners: corners beyond the window
                // or the depth range still shadow pages inside it, so they must widen the rect
                // instead of being dropped.
                let mut uv_min = Vec2::splat(f32::MAX);
                let mut uv_max = Vec2::splat(f32::MIN);
                let mut projected = false;
                for corner in aabb_corners(aabb) {
                    let clip = level.view_proj * corner.extend(1.0);
                    if clip.w <= 1e-4 {
                        continue;
                    }
                    let uv = Vec2::new(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
                    uv_min = uv_min.min(uv);
                    uv_max = uv_max.max(uv);
                    projected = true;
                }
                if !projected
                    || uv_max.x < 0.0
                    || uv_max.y < 0.0
                    || uv_min.x > 1.0
                    || uv_min.y > 1.0
                {
                    continue;
                }
                let pps_f = pps as f32;
                let x0 = (uv_min.x * pps_f).floor().clamp(0.0, pps_f) as u32;
                let y0 = (uv_min.y * pps_f).floor().clamp(0.0, pps_f) as u32;
                let x1 = (uv_max.x * pps_f).ceil().clamp(0.0, pps_f) as u32;
                let y1 = (uv_max.y * pps_f).ceil().clamp(0.0, pps_f) as u32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let entry = pt.get(x, y);
                        if entry & PAGE_VALID_BIT == 0 {
                            continue;
                        }
                        pt.set(x, y, entry & !PAGE_CACHED_BIT);
                        // Re-render now, not whenever the (delayed) screen feedback asks again:
                        // otherwise the stale shadow of a moving caster lingers for a few frames.
                        let flat = lvl_idx * pages_per_level + (y * pps + x) as usize;
                        self.forced[flat / 32] |= 1 << (flat % 32);
                    }
                }
            }
        }
    }

    /// Consumes the visibility pass' requested-page bitfield:
    ///
    /// 1. allocates physical pages for requested-but-invalid entries,
    /// 2. collects dirty pages via
    ///    [`DirtyPageCollector::collect`](super::cache::DirtyPageCollector::collect),
    /// 3. marks them cached (the caller renders them during this frame).
    ///
    /// Returns `(level, page_x, page_y, phys_idx)` tuples ready to be turned
    /// into [`VsmPageDrawInfo`](super::depth_pass::VsmPageDrawInfo)s.
    pub fn requested_dirty_pages(&mut self, requested_bits: &[u32]) -> Vec<DirtyPage> {
        self.frame += 1;
        let frame = self.frame;

        // Pages invalidated by moving casters count as requested this frame.
        let mut merged = requested_bits.to_vec();
        if merged.len() < self.forced.len() {
            merged.resize(self.forced.len(), 0);
        }
        for (word, forced) in merged.iter_mut().zip(&self.forced) {
            *word |= *forced;
        }
        self.forced.fill(0);
        let requested_bits = merged.as_slice();

        // Pass 1: stamp every requested page, so the eviction below can never take a page that
        // is needed this frame.
        let mut bit_offset = 0usize;
        for (li, pt) in self.tables.iter().enumerate() {
            for i in 0..(pt.pages_x * pt.pages_y) as usize {
                if bit_requested(requested_bits, bit_offset + i) {
                    self.last_used[li][i] = frame;
                }
            }
            bit_offset += (pt.pages_x * pt.pages_y) as usize;
        }

        // Pass 2: back requested pages with physical pages, evicting the least recently
        // requested page when the pool is exhausted.
        let mut bit_offset = 0usize;
        for li in 0..self.tables.len() {
            let (nx, ny) = (self.tables[li].pages_x, self.tables[li].pages_y);
            for y in 0..ny {
                for x in 0..nx {
                    let flat = bit_offset + (y * nx + x) as usize;
                    if !bit_requested(requested_bits, flat)
                        || self.tables[li].get(x, y) & PAGE_VALID_BIT != 0
                    {
                        continue;
                    }
                    match self
                        .phys_pool
                        .alloc()
                        .or_else(|| self.evict_least_recent(frame))
                    {
                        Some(phys) => {
                            self.tables[li].set(x, y, (phys as u32) | PAGE_VALID_BIT);
                            self.stats.pages_allocated += 1;
                        }
                        None => self.stats.pages_overflow += 1,
                    }
                }
            }
            bit_offset += (nx * ny) as usize;
        }

        // Pass 3: pages nobody has asked for in a while go back to the pool.
        self.release_stale_pages(frame);

        let dirty = DirtyPageCollector::collect(&self.tables, requested_bits);
        self.stats.pages_dirty = dirty.len();
        dirty
    }

    /// Frees the valid page that was requested longest ago (never one requested in `frame`)
    /// and returns its physical page.
    fn evict_least_recent(&mut self, frame: u64) -> Option<u16> {
        let mut best: Option<(usize, usize, u64)> = None;
        for (li, pt) in self.tables.iter().enumerate() {
            for (i, &entry) in pt.entries.iter().enumerate() {
                if entry & PAGE_VALID_BIT == 0 {
                    continue;
                }
                let used = self.last_used[li][i];
                if used < frame && best.is_none_or(|(_, _, b)| used < b) {
                    best = Some((li, i, used));
                }
            }
        }
        let (li, i, _) = best?;
        let entry = std::mem::take(&mut self.tables[li].entries[i]);
        self.stats.pages_freed += 1;
        Some(phys_page(entry))
    }

    fn release_stale_pages(&mut self, frame: u64) {
        for (li, pt) in self.tables.iter_mut().enumerate() {
            for (i, entry) in pt.entries.iter_mut().enumerate() {
                if *entry & PAGE_VALID_BIT != 0
                    && frame.saturating_sub(self.last_used[li][i]) > STALE_PAGE_FRAMES
                {
                    self.phys_pool.free(phys_page(*entry));
                    self.stats.pages_freed += 1;
                    *entry = 0;
                }
            }
        }
    }

    /// Mark pages as cached after the GPU depth pass has rendered them.
    pub fn mark_pages_cached(&mut self, pages: &[DirtyPage]) {
        for &(level, x, y, _) in pages {
            self.tables[level as usize].mark_cached(x, y);
        }
    }

    /// Packed request bits for clipmap pages that overlap `aabbs`.
    ///
    /// UV matches the PBR lookup (wgpu framebuffer Y: `uv.y = 0.5 - ndc.y * 0.5`).
    /// Used instead of "request every page" so LightMoved does not dirty 2048 pages.
    pub fn request_bits_covering_aabbs(&self, aabbs: &[redox_math::Aabb]) -> Vec<u32> {
        let pages_per_level = (VSM_PAGES_PER_SIDE * VSM_PAGES_PER_SIDE) as usize;
        let n_bits = self.levels.len() * pages_per_level;
        let mut bits = vec![0u32; n_bits.div_ceil(32).max(1)];
        let pps = VSM_PAGES_PER_SIDE;
        let pps_f = pps as f32;
        for (li, level) in self.levels.iter().enumerate() {
            for aabb in aabbs {
                let mut uv_min = Vec2::splat(f32::MAX);
                let mut uv_max = Vec2::splat(f32::MIN);
                let mut any = false;
                for x in [aabb.min.x, aabb.max.x] {
                    for y in [aabb.min.y, aabb.max.y] {
                        for z in [aabb.min.z, aabb.max.z] {
                            let clip = level.view_proj * Vec4::new(x, y, z, 1.0);
                            if clip.w.abs() < 1e-8 {
                                continue;
                            }
                            let ndc = Vec3::new(clip.x, clip.y, clip.z) / clip.w;
                            if !(0.0..=1.0).contains(&ndc.z) {
                                continue;
                            }
                            let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
                            uv_min = uv_min.min(uv);
                            uv_max = uv_max.max(uv);
                            any = true;
                        }
                    }
                }
                if !any {
                    continue;
                }
                let x0 = ((uv_min.x * pps_f).floor() as i32).clamp(0, pps as i32 - 1) as u32;
                let y0 = ((uv_min.y * pps_f).floor() as i32).clamp(0, pps as i32 - 1) as u32;
                let x1 = ((uv_max.x * pps_f).floor() as i32).clamp(0, pps as i32 - 1) as u32;
                let y1 = ((uv_max.y * pps_f).floor() as i32).clamp(0, pps as i32 - 1) as u32;
                for py in y0..=y1 {
                    for px in x0..=x1 {
                        let flat = li * pages_per_level + (py * pps + px) as usize;
                        bits[flat / 32] |= 1 << (flat % 32);
                    }
                }
            }
        }
        bits
    }

    /// Request bits for only the `coarse_levels` coarsest levels that overlap `aabbs`.
    ///
    /// Coarse levels need few pages and always give a lookup something to fall back on while
    /// GPU visibility feedback (fine pages) is still a frame or two behind.
    pub fn coarse_fallback_bits(
        &self,
        aabbs: &[redox_math::Aabb],
        coarse_levels: usize,
    ) -> Vec<u32> {
        let mut bits = self.request_bits_covering_aabbs(aabbs);
        let first_level = self.levels.len().saturating_sub(coarse_levels);
        let pages_per_level = (VSM_PAGES_PER_SIDE * VSM_PAGES_PER_SIDE) as usize;
        for flat in 0..first_level * pages_per_level {
            bits[flat / 32] &= !(1 << (flat % 32));
        }
        bits
    }

    /// Packed request bits for clipmap pages that cover `world_points`.
    ///
    /// UV matches the PBR lookup (wgpu framebuffer Y: `uv.y = 0.5 - ndc.y * 0.5`).
    /// Used instead of "request every page" so LightMoved does not dirty 2048 pages.
    pub fn request_bits_covering(&self, world_points: &[Vec3]) -> Vec<u32> {
        let pages_per_level = (VSM_PAGES_PER_SIDE * VSM_PAGES_PER_SIDE) as usize;
        let n_bits = self.levels.len() * pages_per_level;
        let mut bits = vec![0u32; n_bits.div_ceil(32).max(1)];
        let pps = VSM_PAGES_PER_SIDE as f32;
        for (li, level) in self.levels.iter().enumerate() {
            for &p in world_points {
                let clip = level.view_proj * Vec4::new(p.x, p.y, p.z, 1.0);
                if clip.w.abs() < 1e-8 {
                    continue;
                }
                let ndc = Vec3::new(clip.x, clip.y, clip.z) / clip.w;
                let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
                if !(0.0..=1.0).contains(&uv.x) || !(0.0..=1.0).contains(&uv.y) {
                    continue;
                }
                if !(0.0..=1.0).contains(&ndc.z) {
                    continue;
                }
                let px = ((uv.x * pps) as u32).min(VSM_PAGES_PER_SIDE - 1);
                let py = ((uv.y * pps) as u32).min(VSM_PAGES_PER_SIDE - 1);
                let flat = li * pages_per_level + (py * VSM_PAGES_PER_SIDE + px) as usize;
                bits[flat / 32] |= 1 << (flat % 32);
            }
        }
        bits
    }

    /// View-projection that maps a single page's world slice to NDC `[-1, 1]`.
    ///
    /// Sampling uses level `view_proj` → UV → `fract(uv * pages_per_side)` into
    /// the physical page; rendering must use this page-local matrix so that
    /// same fract UV hits the matching texels (not a thumbnail of the whole level).
    ///
    /// Ortho is expanded by **1 texel** on each side so adjacent pages overlap.
    /// Without that, page edges stay at clear-depth (far) and show up as bright
    /// seam lines on receivers.
    pub fn page_view_proj(&self, level: usize, page_x: u32, page_y: u32) -> Mat4 {
        let level_vp = self.levels[level].view_proj;
        let pps = VSM_PAGES_PER_SIDE as f32;
        let page_px = self.config.page_px as f32;
        let px = page_x as f32;
        let py = page_y as f32;
        // Cover (page_px+2)/page_px of the logical page so edge texels get real depth.
        let s = pps * page_px / (page_px + 2.0);
        let cx = 2.0 * (px + 0.5) / pps - 1.0;
        // wgpu UV.y = 0.5 - ndc.y*0.5 → page row 0 is level NDC +Y.
        let cy = 1.0 - 2.0 * (py + 0.5) / pps;
        let scale_bias = Mat4::from_translation(Vec3::new(-s * cx, -s * cy, 0.0))
            * Mat4::from_scale(Vec3::new(s, s, 1.0));
        scale_bias * level_vp
    }

    /// World-space AABB covered by one virtual page (for caster culling).
    pub fn page_world_aabb(&self, level: usize, page_x: u32, page_y: u32) -> Aabb {
        let inv = self.page_view_proj(level, page_x, page_y).inverse();
        let corners = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];
        let mut min_w = Vec3::splat(f32::INFINITY);
        let mut max_w = Vec3::splat(f32::NEG_INFINITY);
        for c in corners {
            let w = inv.transform_point3(c);
            min_w = min_w.min(w);
            max_w = max_w.max(w);
        }
        Aabb {
            min: min_w,
            max: max_w,
        }
    }

    /// Builds the flat GPU uniform consumed by the PBR shader's
    /// `vsm_shadow()` (binding 24) — layout mirrors WGSL `VsmGpuInfo`.
    ///
    /// The same `levels` array is also valid input for the visibility pass'
    /// `array<ClipmapLevel>` storage binding (identical first 76 bytes per
    /// element; the visibility shader reads the trailing word as `_pad`).
    pub fn gpu_info(&self, smrt: &SmrtConfig) -> VsmGpuInfo {
        let mut info = VsmGpuInfo {
            level_count: self.levels.len() as u32,
            page_px: self.config.page_px,
            phys_page_count: self.phys_pool.capacity() as u32,
            phys_pages_per_side: (self.phys_pool.capacity() as f32).sqrt().ceil().max(1.0) as u32,
            ray_count: smrt.ray_count,
            samples_per_ray: smrt.samples_per_ray,
            source_angle: smrt.source_angle,
            source_radius: smrt.source_radius,
            max_trace_dist: smrt.max_trace_dist,
            ..Zeroable::zeroed()
        };

        for (i, level) in self.levels.iter().take(VSM_MAX_LEVELS).enumerate() {
            info.levels[i] = VsmLevelGpu {
                view_proj: level.view_proj.to_cols_array_2d(),
                world_half_extent: level.world_half_extent,
                page_table_offset: (i * VSM_PAGE_TABLE_WORDS_PER_LEVEL) as u32,
                pages_per_side: VSM_PAGES_PER_SIDE,
                texel_world: level.texel_world,
            };
        }
        for (i, pt) in self.tables.iter().take(VSM_MAX_LEVELS).enumerate() {
            let offset = i * VSM_PAGE_TABLE_WORDS_PER_LEVEL;
            let n = VSM_PAGE_TABLE_WORDS_PER_LEVEL.min(pt.entries.len());
            info.page_tables[offset..offset + n].copy_from_slice(&pt.entries[..n]);
        }
        info
    }
}

/// Shifts one page table by `(dx, dy)` whole pages and returns the physical
/// indices of entries that fell out of the window (the caller recycles them
/// into its allocator).
///
/// Sliding the window origin by `+d` pages shifts every world region's
/// virtual index by `−d`: the content of old cell `(tx + dx, ty + dy)`
/// becomes cell `(tx, ty)` in the shifted table.
fn shift_entries(pt: &mut VirtualPageTable, dx: i32, dy: i32) -> Vec<u16> {
    let nx = pt.pages_x as i32;
    let ny = pt.pages_y as i32;
    let old = pt.entries.clone();

    for ty in 0..ny {
        for tx in 0..nx {
            let (sx, sy) = (tx + dx, ty + dy);
            let value = if sx >= 0 && sx < nx && sy >= 0 && sy < ny {
                old[(sy * nx + sx) as usize]
            } else {
                0
            };
            pt.entries[(ty * nx + tx) as usize] = value;
        }
    }

    // An old entry survives iff its destination `(t − d)` stays inside the
    // window; otherwise it falls out exactly once.
    let mut dropped = Vec::new();
    for (i, &entry) in old.iter().enumerate() {
        if entry & PAGE_VALID_BIT == 0 {
            continue;
        }
        let tx = (i as i32 % nx) - dx;
        let ty = (i as i32 / nx) - dy;
        if tx < 0 || tx >= nx || ty < 0 || ty >= ny {
            dropped.push(phys_page(entry));
        }
    }
    dropped
}

/// The eight corners of `aabb`.
fn aabb_corners(aabb: &Aabb) -> [Vec3; 8] {
    let (a, b) = (aabb.min, aabb.max);
    [
        Vec3::new(a.x, a.y, a.z),
        Vec3::new(b.x, a.y, a.z),
        Vec3::new(a.x, b.y, a.z),
        Vec3::new(b.x, b.y, a.z),
        Vec3::new(a.x, a.y, b.z),
        Vec3::new(b.x, a.y, b.z),
        Vec3::new(a.x, b.y, b.z),
        Vec3::new(b.x, b.y, b.z),
    ]
}

/// Shifts a square `n x n` grid by `(dx, dy)` cells with the same mapping as [`shift_entries`]
/// (new `(tx, ty)` takes old `(tx + dx, ty + dy)`, cells shifted in from outside become 0).
fn shift_grid(grid: &mut [u64], n: i32, dx: i32, dy: i32) {
    let old = grid.to_vec();
    for ty in 0..n {
        for tx in 0..n {
            let (sx, sy) = (tx + dx, ty + dy);
            grid[(ty * n + tx) as usize] = if (0..n).contains(&sx) && (0..n).contains(&sy) {
                old[(sy * n + sx) as usize]
            } else {
                0
            };
        }
    }
}

/// Reads one bit from the visibility pass' packed request bitfield.
#[inline]
fn bit_requested(requested_bits: &[u32], flat: usize) -> bool {
    let word = flat / 32;
    let bit = flat % 32;
    word < requested_bits.len() && (requested_bits[word] & (1 << bit)) != 0
}

/// GPU mirror of one clipmap level — byte-compatible with
/// `shaders/vsm_visibility.wgsl::ClipmapLevel` (80 bytes; the final
/// `texel_world` occupies that shader's `_pad` slot).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VsmLevelGpu {
    pub view_proj: [[f32; 4]; 4],
    pub world_half_extent: f32,
    pub page_table_offset: u32,
    pub pages_per_side: u32,
    pub texel_world: f32,
}

/// Flat words per level in the combined page-table array.
/// Fixed at 256 (16×16 max) to keep `VsmGpuInfo` size stable (8896 B) for WGSL,
/// even when `VSM_PAGES_PER_SIDE` is reduced (e.g. 4) to cut mosaic.
pub const VSM_PAGE_TABLE_WORDS_PER_LEVEL: usize = 256;

/// Total words in [`VsmGpuInfo::page_tables`].
pub const VSM_PAGE_TABLE_WORDS: usize = VSM_MAX_LEVELS * VSM_PAGE_TABLE_WORDS_PER_LEVEL;
/// GPU mirror of `pbr.wgsl::VsmGpuInfo` (binding 24, storage read-only).
///
/// Layout contract (bytes): header 48 → `levels[8]` @ 48 (80 B each) →
/// `page_tables[2048]` @ 688. Do not reorder fields without updating the
/// WGSL side.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VsmGpuInfo {
    pub level_count: u32,
    pub page_px: u32,
    /// Physical pages in the pool.
    pub phys_page_count: u32,
    /// Atlas pages along one axis (√pool, rounded up).
    pub phys_pages_per_side: u32,
    /// SMRT params (mirror of [`SmrtUniform`](super::smrt::SmrtUniform)).
    pub ray_count: u32,
    pub samples_per_ray: u32,
    pub source_angle: f32,
    pub source_radius: f32,
    pub max_trace_dist: f32,
    pub _pad0: [f32; 3],
    /// 16-byte alignment pad: WGSL vec3<f32> has size 16, so the following
    /// array<..mat4..> must start at a 16-aligned offset (shader struct = 8896 B).
    pub _pad_align: [f32; 4],
    pub levels: [VsmLevelGpu; VSM_MAX_LEVELS],
    /// Concatenated page-table entries, `level * 256 + y * 16 + x`.
    pub page_tables: [u32; VSM_PAGE_TABLE_WORDS],
}

#[cfg(test)]
mod tests {
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
        let expected =
            header_bytes + 8 * std::mem::size_of::<VsmLevelGpu>() + VSM_PAGE_TABLE_WORDS * 4;
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
}

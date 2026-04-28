//! VSM support for local (spot and point) lights.
//!
//! * **Spot lights** use a single perspective VSM with one page table.
//! * **Point lights** use six perspective VSMs (one per cube face) sharing
//!   the same physical atlas.

use bytemuck::{Pod, Zeroable};
use redox_math::{Mat4, Vec3};

use super::page_table::{
    PhysicalPageAllocator, VirtualPageTable, PAGE_CACHED_BIT, PAGE_PHYS_MASK, PAGE_VALID_BIT,
};
use super::shadow_atlas::ShadowAtlas;
use super::depth_pass::VsmPageDrawInfo;

// ── Cube face ───────────────────────────────────────────────────────────────

/// Index of a cube-map face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CubeFace {
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}

impl CubeFace {
    pub const ALL: [CubeFace; 6] = [
        CubeFace::PosX, CubeFace::NegX,
        CubeFace::PosY, CubeFace::NegY,
        CubeFace::PosZ, CubeFace::NegZ,
    ];

    pub fn forward_up(self) -> (Vec3, Vec3) {
        match self {
            CubeFace::PosX => (Vec3::X, Vec3::Y),
            CubeFace::NegX => (Vec3::NEG_X, Vec3::Y),
            CubeFace::PosY => (Vec3::Y, Vec3::NEG_Z),
            CubeFace::NegY => (Vec3::NEG_Y, Vec3::Z),
            CubeFace::PosZ => (Vec3::Z, Vec3::Y),
            CubeFace::NegZ => (Vec3::NEG_Z, Vec3::Y),
        }
    }

    /// Returns true if this face should be rendered into the shadow map.
    /// Always true: we need all 6 faces for correct point-light shadows.
    /// The previous hemisphere culling (fwd.dot(to_cam) > -0.3) incorrectly
    /// skipped NegY when the camera was above the light — but NegY looks
    /// down and contains the floor/cubes, so shadows were missing.
    pub fn is_visible_from(self, _light_pos: Vec3, _camera_pos: Vec3) -> bool {
        true
    }
}

// ── Spot light VSM ──────────────────────────────────────────────────────────

/// VSM data for a single spot light.
pub struct SpotLightVsm {
    pub view_proj: Mat4,
    pub page_table: VirtualPageTable,
    pub fov_rad: f32,
    pub source_radius: f32,
    pub position: Vec3,
    pub direction: Vec3,
    pub range: f32,
}

impl SpotLightVsm {
    pub fn new(pages_per_side: u32) -> Self {
        Self {
            view_proj: Mat4::IDENTITY,
            page_table: VirtualPageTable::new(pages_per_side, pages_per_side),
            fov_rad: std::f32::consts::FRAC_PI_2,
            source_radius: 0.05,
            position: Vec3::ZERO,
            direction: Vec3::NEG_Z,
            range: 30.0,
        }
    }

    pub fn update_matrix(&mut self) {
        let (fwd, up) = (self.direction, Vec3::Y);
        let target = self.position + fwd;
        let view = Mat4::look_at_rh(self.position, target, up);
        let proj = Mat4::perspective_rh(self.fov_rad, 1.0, 0.1, self.range);
        self.view_proj = proj * view;
    }

    /// Allocate pages, collect dirty draw infos. Returns count of dirty pages added.
    pub fn process_pages(
        &mut self,
        allocator: &mut PhysicalPageAllocator,
        atlas: &ShadowAtlas,
        frame: u64,
        dirty_out: &mut Vec<VsmPageDrawInfo>,
        stats_alloc: &mut u32,
        stats_cached: &mut u32,
        stats_overflow: &mut u32,
    ) -> u32 {
        let pt = &mut self.page_table;
        let mut rendered = 0u32;
        for y in 0..pt.pages_y {
            for x in 0..pt.pages_x {
                let entry = pt.get(x, y);
                if entry & PAGE_VALID_BIT == 0 {
                    if let Some(phys) = allocator.allocate(frame) {
                        pt.set(x, y, (phys as u32) | PAGE_VALID_BIT);
                        *stats_alloc += 1;
                    } else {
                        allocator.evict_lru(32, frame);
                        if let Some(phys) = allocator.allocate(frame) {
                            pt.set(x, y, (phys as u32) | PAGE_VALID_BIT);
                            *stats_alloc += 1;
                        } else {
                            *stats_overflow += 1;
                        }
                    }
                } else if entry & PAGE_CACHED_BIT != 0 {
                    allocator.touch((entry & PAGE_PHYS_MASK) as u16, frame);
                    *stats_cached += 1;
                }
                // Collect dirty
                let entry = pt.get(x, y);
                if entry & PAGE_VALID_BIT != 0 && entry & PAGE_CACHED_BIT == 0 {
                    let phys = (entry & PAGE_PHYS_MASK) as u16;
                    let (ox, oy) = atlas.page_offset(phys);
                    dirty_out.push(VsmPageDrawInfo {
                        view_proj: self.view_proj.to_cols_array_2d(),
                        atlas_offset_x: ox as f32,
                        atlas_offset_y: oy as f32,
                        page_size_px: atlas.page_size_px as f32,
                        atlas_size_px: atlas.size_px as f32,
                    });
                    pt.set(x, y, entry | PAGE_CACHED_BIT);
                    rendered += 1;
                }
            }
        }
        rendered
    }
}

// ── Point light VSM ─────────────────────────────────────────────────────────

/// VSM data for a point light (six cube faces).
pub struct PointLightVsm {
    pub position: Vec3,
    pub radius: f32,
    pub source_radius: f32,
    pub face_view_proj: [Mat4; 6],
    pub face_page_table: [VirtualPageTable; 6],
}

impl PointLightVsm {
    pub fn new(position: Vec3, radius: f32, pages_per_side: u32) -> Self {
        Self {
            position,
            radius,
            source_radius: 0.05,
            face_view_proj: [Mat4::IDENTITY; 6],
            face_page_table: std::array::from_fn(|_| {
                VirtualPageTable::new(pages_per_side, pages_per_side)
            }),
        }
    }

    /// Builds view-proj for each cube face. Column-major for WGSL; RH perspective, depth [0,1].
    /// Uses glam (via redox_math) so that w_axis.w = 1.0 (translation column) and clip.w is correct.
    pub fn update_matrices(&mut self) {
        let near = 0.1_f32;
        let far = self.radius.max(0.5);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
        for face in CubeFace::ALL {
            let (fwd, up) = face.forward_up();
            let target = self.position + fwd;
            let view = Mat4::look_at_rh(self.position, target, up);
            self.face_view_proj[face as usize] = proj * view;
        }
    }

    /// Like `SpotLightVsm::process_pages` but for all 6 faces.
    /// `camera_pos` is used for face culling.
    pub fn process_pages(
        &mut self,
        allocator: &mut PhysicalPageAllocator,
        atlas: &ShadowAtlas,
        frame: u64,
        camera_pos: Vec3,
        dirty_out: &mut Vec<VsmPageDrawInfo>,
        stats_alloc: &mut u32,
        stats_cached: &mut u32,
        stats_overflow: &mut u32,
    ) -> u32 {
        let mut total_rendered = 0u32;
        for face in CubeFace::ALL {
            if !face.is_visible_from(self.position, camera_pos) {
                continue;
            }
            let fi = face as usize;
            let vp = self.face_view_proj[fi];
            let pt = &mut self.face_page_table[fi];
            for y in 0..pt.pages_y {
                for x in 0..pt.pages_x {
                    let entry = pt.get(x, y);
                    if entry & PAGE_VALID_BIT == 0 {
                        if let Some(phys) = allocator.allocate(frame) {
                            pt.set(x, y, (phys as u32) | PAGE_VALID_BIT);
                            *stats_alloc += 1;
                        } else {
                            allocator.evict_lru(32, frame);
                            if let Some(phys) = allocator.allocate(frame) {
                                pt.set(x, y, (phys as u32) | PAGE_VALID_BIT);
                                *stats_alloc += 1;
                            } else {
                                *stats_overflow += 1;
                            }
                        }
                    } else if entry & PAGE_CACHED_BIT != 0 {
                        allocator.touch((entry & PAGE_PHYS_MASK) as u16, frame);
                        *stats_cached += 1;
                    }
                    let entry = pt.get(x, y);
                    if entry & PAGE_VALID_BIT != 0 && entry & PAGE_CACHED_BIT == 0 {
                        let phys = (entry & PAGE_PHYS_MASK) as u16;
                        let (ox, oy) = atlas.page_offset(phys);
                        dirty_out.push(VsmPageDrawInfo {
                            view_proj: vp.to_cols_array_2d(),
                            atlas_offset_x: ox as f32,
                            atlas_offset_y: oy as f32,
                            page_size_px: atlas.page_size_px as f32,
                            atlas_size_px: atlas.size_px as f32,
                        });
                        pt.set(x, y, entry | PAGE_CACHED_BIT);
                        total_rendered += 1;
                    }
                }
            }
        }
        total_rendered
    }

    /// Collect all page-table entries (all 6 faces) into a flat vec for GPU upload.
    pub fn collect_page_table_entries(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for pt in &self.face_page_table {
            out.extend_from_slice(&pt.entries);
        }
        out
    }

    pub fn invalidate_all(&mut self) {
        for pt in &mut self.face_page_table {
            pt.invalidate_all();
        }
    }
}

// ── GPU structs ─────────────────────────────────────────────────────────────

/// GPU-uploadable descriptor for one local-light VSM (or one face of a point light).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LocalLightFaceGpu {
    pub view_proj: [[f32; 4]; 4],
    pub page_table_offset: u32,
    pub pages_per_side: u32,
    pub _pad: [u32; 2],
}

/// Maximum local-light VSM faces the engine ships to the GPU in one frame.
/// 32 spot lights + 8 point lights × 6 faces = 80. Round up.
pub const MAX_LOCAL_VSM_FACES: usize = 128;

/// GPU-uploadable info for all local-light VSMs combined.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LocalLightVsmInfoGpu {
    pub num_spot_faces: u32,
    pub num_point_faces: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

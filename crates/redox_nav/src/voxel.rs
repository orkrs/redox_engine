//! Conversion of agent parameters from metres to voxel units.
//!
//! Pure arithmetic: no backend types. The baker turns these numbers into rerecast settings.

use crate::data::AgentParams;

/// Smallest walkable height the Recast voxel pipeline accepts, in voxels.
const MIN_WALKABLE_HEIGHT: u16 = 3;
/// Tolerance that absorbs f32 rounding before `ceil`/`floor` on exact multiples.
const ROUNDING_EPSILON: f32 = 1e-4;
/// Contour simplification error in voxels (Recast's recommended value).
const MAX_SIMPLIFICATION_ERROR: f32 = 1.3;
/// Longest contour edge, in agent radii of voxels.
const EDGE_MAX_LEN_FACTOR: u16 = 8;
/// Minimum region area in voxels (8 x 8).
const MIN_REGION_AREA: u16 = 8 * 8;
/// Regions smaller than this are merged into neighbours (20 x 20).
const MERGE_REGION_AREA: u16 = 20 * 20;
/// Largest polygon the baker produces; the `.nav` format stores at most 6 indices.
const MAX_VERTICES_PER_POLYGON: u16 = 6;

/// Voxel-unit settings derived from [`AgentParams`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelConfig {
    /// Cell size on the xz-plane, metres.
    pub cell_size: f32,
    /// Cell size along y, metres.
    pub cell_height: f32,
    /// Erosion distance from obstacles, in cells (at least 1).
    pub walkable_radius: u16,
    /// Agent height in cells (at least 3).
    pub walkable_height: u16,
    /// Maximum ledge height in cells.
    pub walkable_climb: u16,
    /// Padding around a tile, in cells.
    pub border_size: u16,
    /// Steepest walkable slope in radians.
    pub walkable_slope_rad: f32,
    /// Contour simplification error in cells.
    pub max_simplification_error: f32,
    /// Longest contour edge in cells (0 disables splitting).
    pub max_edge_len: u16,
    /// Minimum region area in cells.
    pub min_region_area: u16,
    /// Regions up to this area are merged into neighbours, in cells.
    pub merge_region_area: u16,
    /// Maximum vertices per polygon.
    pub max_vertices_per_polygon: u16,
}

/// Derives voxel settings for `agent`.
///
/// The cell size is `radius / 2` and the cell height is half of that, which gives an
/// erosion radius of two cells for any agent.
pub fn voxel_config(agent: &AgentParams) -> VoxelConfig {
    let cell_size = agent.radius / 2.0;
    let cell_height = cell_size / 2.0;
    let walkable_radius = ((agent.radius / cell_size - ROUNDING_EPSILON).ceil() as u16).max(1);
    let walkable_height =
        ((agent.height / cell_height - ROUNDING_EPSILON).ceil() as u16).max(MIN_WALKABLE_HEIGHT);
    let walkable_climb = (agent.max_step / cell_height + ROUNDING_EPSILON).floor() as u16;
    VoxelConfig {
        cell_size,
        cell_height,
        walkable_radius,
        walkable_height,
        walkable_climb,
        border_size: walkable_radius + 3,
        walkable_slope_rad: agent.max_slope_deg.to_radians(),
        max_simplification_error: MAX_SIMPLIFICATION_ERROR,
        max_edge_len: walkable_radius * EDGE_MAX_LEN_FACTOR,
        min_region_area: MIN_REGION_AREA,
        merge_region_area: MERGE_REGION_AREA,
        max_vertices_per_polygon: MAX_VERTICES_PER_POLYGON,
    }
}

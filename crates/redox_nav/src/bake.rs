//! Baking: level triangles to tiled navmesh tiles, using the rerecast (Recast) pipeline.
//!
//! The rerecast types never leave this module. Each tile is built on its own: the triangles
//! that touch the tile plus a border are rasterised, and only polygons whose centre lies in
//! the tile core are kept.

use rerecast::{Aabb3d, AreaType, BuildContoursFlags, HeightfieldBuilder, PolygonNavmesh, TriMesh};

use crate::data::{AgentParams, NavAreaId, NavData, NavPolygon, NavTile};
use crate::error::BakeError;
use crate::voxel::{VoxelConfig, voxel_config};

/// Triangle soup in world space with one area per triangle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavGeometry {
    /// World-space vertices (x, y up, z).
    pub vertices: Vec<[f32; 3]>,
    /// Triangles as vertex indices.
    pub triangles: Vec<[u32; 3]>,
    /// Area of each triangle; `areas.len() == triangles.len()`.
    pub areas: Vec<NavAreaId>,
}

/// Bakes `geometry` into tiles of `tile_size` metres for `agent`.
///
/// # Errors
///
/// [`BakeError::BadTileSize`] for a non-positive tile size, [`BakeError::EmptyGeometry`]
/// for no triangles, [`BakeError::BadGeometry`] when the arrays disagree, and
/// [`BakeError::Backend`] when rerecast fails.
pub fn bake(
    geometry: &NavGeometry,
    agent: &AgentParams,
    tile_size: f32,
) -> Result<NavData, BakeError> {
    if !(tile_size.is_finite() && tile_size > 0.0) {
        return Err(BakeError::BadTileSize);
    }
    validate(geometry)?;
    let config = voxel_config(agent);
    let bounds = Bounds::of(&geometry.vertices);
    let border = f32::from(config.border_size) * config.cell_size;

    let (x0, x1) = (
        tile_index(bounds.min[0], tile_size),
        tile_index(bounds.max[0], tile_size),
    );
    let (z0, z1) = (
        tile_index(bounds.min[2], tile_size),
        tile_index(bounds.max[2], tile_size),
    );
    let mut tiles = Vec::new();
    for tz in z0..=z1 {
        for tx in x0..=x1 {
            let spec = TileSpec {
                coord: (tx, tz),
                tile_size,
                border,
                bounds,
            };
            if let Some(tile) = bake_tile(geometry, &config, &spec)? {
                tiles.push(tile);
            }
        }
    }
    Ok(NavData {
        agent: *agent,
        tile_size,
        tiles,
    })
}

fn validate(geometry: &NavGeometry) -> Result<(), BakeError> {
    if geometry.triangles.is_empty() {
        return Err(BakeError::EmptyGeometry);
    }
    if geometry.areas.len() != geometry.triangles.len() {
        return Err(BakeError::BadGeometry("one area per triangle is required"));
    }
    let vertex_count = geometry.vertices.len();
    let indices_ok = geometry
        .triangles
        .iter()
        .all(|tri| tri.iter().all(|&i| (i as usize) < vertex_count));
    if !indices_ok {
        return Err(BakeError::BadGeometry("triangle index out of range"));
    }
    Ok(())
}

/// Axis-aligned bounds of the vertices.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
}

impl Bounds {
    fn of(vertices: &[[f32; 3]]) -> Self {
        let mut bounds = Self {
            min: [f32::MAX; 3],
            max: [f32::MIN; 3],
        };
        for vertex in vertices {
            for (axis, &value) in vertex.iter().enumerate() {
                bounds.min[axis] = bounds.min[axis].min(value);
                bounds.max[axis] = bounds.max[axis].max(value);
            }
        }
        bounds
    }
}

/// Tile grid index of a coordinate.
fn tile_index(coordinate: f32, tile_size: f32) -> i32 {
    (coordinate / tile_size).floor() as i32
}

/// Everything one tile bake needs besides the geometry and the voxel settings.
struct TileSpec {
    coord: (i32, i32),
    tile_size: f32,
    border: f32,
    bounds: Bounds,
}

fn bake_tile(
    geometry: &NavGeometry,
    config: &VoxelConfig,
    spec: &TileSpec,
) -> Result<Option<NavTile>, BakeError> {
    let core_min = [
        spec.coord.0 as f32 * spec.tile_size,
        spec.coord.1 as f32 * spec.tile_size,
    ];
    let core_max = [core_min[0] + spec.tile_size, core_min[1] + spec.tile_size];
    let outer_min = [core_min[0] - spec.border, core_min[1] - spec.border];
    let outer_max = [core_max[0] + spec.border, core_max[1] + spec.border];

    let Some(trimesh) = tile_trimesh(geometry, outer_min, outer_max, config) else {
        return Ok(None);
    };
    let aabb = tile_aabb(outer_min, outer_max, &spec.bounds);
    let mesh = build_polygon_mesh(trimesh, aabb, config)?;
    extract_tile(&mesh, spec.coord, core_min, core_max)
}

/// Triangles whose XZ box overlaps `[min, max]`, with their area types set for rerecast.
///
/// Walkability (slope) comes from rerecast. Areas are kept by remembering each triangle's
/// own area and writing it back over the walkable ones; steep or not-walkable triangles stay
/// as obstacles with [`AreaType::NOT_WALKABLE`].
fn tile_trimesh(
    geometry: &NavGeometry,
    min: [f32; 2],
    max: [f32; 2],
    config: &VoxelConfig,
) -> Option<TriMesh> {
    let mut mesh = TriMesh::default();
    let mut areas = Vec::new();
    for (tri, area) in geometry.triangles.iter().zip(&geometry.areas) {
        let corners = tri.map(|i| geometry.vertices[i as usize]);
        if !overlaps_xz(&corners, min, max) {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        for corner in corners {
            mesh.vertices.push(corner.into());
        }
        mesh.indices.push([base, base + 1, base + 2].into());
        mesh.area_types.push(AreaType::NOT_WALKABLE);
        areas.push(*area);
    }
    if mesh.indices.is_empty() {
        return None;
    }
    mesh.mark_walkable_triangles(config.walkable_slope_rad);
    for (index, area) in areas.iter().enumerate() {
        if mesh.area_types[index].is_walkable() {
            mesh.area_types[index] = encode_area(*area);
        }
    }
    Some(mesh)
}

fn overlaps_xz(corners: &[[f32; 3]; 3], min: [f32; 2], max: [f32; 2]) -> bool {
    let lo_x = corners.iter().map(|c| c[0]).fold(f32::MAX, f32::min);
    let hi_x = corners.iter().map(|c| c[0]).fold(f32::MIN, f32::max);
    let lo_z = corners.iter().map(|c| c[2]).fold(f32::MAX, f32::min);
    let hi_z = corners.iter().map(|c| c[2]).fold(f32::MIN, f32::max);
    hi_x >= min[0] && lo_x <= max[0] && hi_z >= min[1] && lo_z <= max[1]
}

/// Height box of a tile: the whole level's height plus a margin.
fn tile_aabb(min: [f32; 2], max: [f32; 2], bounds: &Bounds) -> Aabb3d {
    let margin = 1.0;
    let y_min = bounds.min[1] - margin;
    let y_max = bounds.max[1] + margin;
    Aabb3d::new(
        [
            (min[0] + max[0]) / 2.0,
            (y_min + y_max) / 2.0,
            (min[1] + max[1]) / 2.0,
        ],
        [
            (max[0] - min[0]) / 2.0,
            (y_max - y_min) / 2.0,
            (max[1] - min[1]) / 2.0,
        ],
    )
}

fn build_polygon_mesh(
    trimesh: TriMesh,
    aabb: Aabb3d,
    config: &VoxelConfig,
) -> Result<PolygonNavmesh, BakeError> {
    let mut heightfield = HeightfieldBuilder {
        aabb,
        cell_size: config.cell_size,
        cell_height: config.cell_height,
    }
    .build()
    .map_err(backend)?;
    heightfield
        .populate_from_trimesh(trimesh, config.walkable_height, config.walkable_climb)
        .map_err(backend)?;
    let mut compact = heightfield
        .into_compact(config.walkable_height, config.walkable_climb)
        .map_err(backend)?;
    compact.erode_walkable_area(config.walkable_radius);
    compact.build_distance_field();
    compact
        .build_regions(
            config.border_size,
            config.min_region_area,
            config.merge_region_area,
        )
        .map_err(backend)?;
    let contours = compact.build_contours(
        config.max_simplification_error,
        config.max_edge_len,
        BuildContoursFlags::default(),
    );
    contours
        .into_polygon_mesh(config.max_vertices_per_polygon)
        .map_err(backend)
}

/// Keeps the polygons whose centre lies in the tile core and converts them to world space.
fn extract_tile(
    mesh: &PolygonNavmesh,
    coord: (i32, i32),
    core_min: [f32; 2],
    core_max: [f32; 2],
) -> Result<Option<NavTile>, BakeError> {
    let stride = usize::from(mesh.max_vertices_per_polygon);
    if stride == 0 {
        return Ok(None);
    }
    let mut remap: Vec<Option<u16>> = vec![None; mesh.vertices.len()];
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut polygons = Vec::new();
    for (polygon, area_type) in mesh.areas.iter().enumerate() {
        let Some(area) = decode_area(*area_type) else {
            continue;
        };
        let Some(slots) = mesh.polygons.get(polygon * stride..(polygon + 1) * stride) else {
            continue;
        };
        let corners: Vec<usize> = slots
            .iter()
            .take_while(|&&slot| slot != PolygonNavmesh::NO_INDEX)
            .map(|&slot| usize::from(slot))
            .collect();
        if corners.len() < 3 {
            continue;
        }
        let world: Vec<[f32; 3]> = corners
            .iter()
            .filter_map(|&corner| mesh.vertices.get(corner))
            .map(|v| {
                [
                    mesh.aabb.min.x + f32::from(v.x) * mesh.cell_size,
                    mesh.aabb.min.y + f32::from(v.y) * mesh.cell_height,
                    mesh.aabb.min.z + f32::from(v.z) * mesh.cell_size,
                ]
            })
            .collect();
        if world.len() != corners.len() || !centre_in_core(&world, core_min, core_max) {
            continue;
        }
        let mut indices = Vec::with_capacity(corners.len());
        for (&corner, position) in corners.iter().zip(&world) {
            let index = match remap.get(corner).copied().flatten() {
                Some(index) => index,
                None => {
                    let index = u16::try_from(vertices.len())
                        .map_err(|_| BakeError::Backend("tile has too many vertices".into()))?;
                    vertices.push(*position);
                    if let Some(slot) = remap.get_mut(corner) {
                        *slot = Some(index);
                    }
                    index
                }
            };
            indices.push(index);
        }
        polygons.push(NavPolygon { indices, area });
    }
    if polygons.is_empty() {
        return Ok(None);
    }
    Ok(Some(NavTile {
        coord,
        vertices,
        polygons,
    }))
}

fn centre_in_core(world: &[[f32; 3]], core_min: [f32; 2], core_max: [f32; 2]) -> bool {
    let count = world.len() as f32;
    let centre_x = world.iter().map(|p| p[0]).sum::<f32>() / count;
    let centre_z = world.iter().map(|p| p[2]).sum::<f32>() / count;
    centre_x >= core_min[0]
        && centre_x < core_max[0]
        && centre_z >= core_min[1]
        && centre_z < core_max[1]
}

/// Area of a rerecast polygon; `None` for not walkable.
fn decode_area(area: AreaType) -> Option<NavAreaId> {
    if area.0 == 0 {
        None
    } else {
        Some(NavAreaId(area.0 - 1))
    }
}

/// rerecast area for a walkable triangle: zero is reserved for not walkable.
fn encode_area(area: NavAreaId) -> AreaType {
    if area == NavAreaId::NOT_WALKABLE {
        AreaType::NOT_WALKABLE
    } else {
        AreaType(area.0 + 1)
    }
}

fn backend<E: std::fmt::Display>(error: E) -> BakeError {
    BakeError::Backend(error.to_string())
}

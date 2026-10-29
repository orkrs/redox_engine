//! The only module that talks to `landmass`. Its types stay here; the facade converts
//! `redox_math::Vec3` at its boundary.
//!
//! landmass is Z-up and the engine is Y-up, so [`YUp`] swaps Y and Z. The swap mirrors the
//! plane, so polygons are flipped (`FLIP_POLYGONS`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use landmass::{
    Agent, AgentId, AgentState, AnimationLink, AnimationLinkId, Archipelago, ArchipelagoOptions,
    CoordinateSystem, FromAgentRadius, Island, NavigationMesh, PathStep, PermittedAnimationLinks,
    PointSampleDistance3d, SampledPoint, Transform, ValidNavigationMesh, Vec3 as LmVec3,
};
use redox_math::Vec3;

use crate::avoid::AgentStatus;
use crate::data::{NavPolygon, NavTile};
use crate::error::NavError;

/// Coordinate system of the backend: Y-up engine points, landmass Z-up points.
pub(crate) struct YUp;

impl CoordinateSystem for YUp {
    type Coordinate = LmVec3;
    type SampleDistance = PointSampleDistance3d;

    const FLIP_POLYGONS: bool = true;

    fn to_landmass(v: &LmVec3) -> LmVec3 {
        LmVec3::new(v.x, v.z, v.y)
    }

    fn from_landmass(v: &LmVec3) -> LmVec3 {
        LmVec3::new(v.x, v.z, v.y)
    }
}

/// The archipelago used by the facade.
pub(crate) type Arch = Archipelago<YUp>;

/// Converts an engine point to a landmass point (same axes, other type).
pub(crate) fn to_lm(v: Vec3) -> LmVec3 {
    LmVec3::from_array(v.to_array())
}

/// Converts a landmass point back to an engine point.
pub(crate) fn from_lm(v: LmVec3) -> Vec3 {
    Vec3::from_array(v.to_array())
}

/// Options of an archipelago for agents of `radius`.
pub(crate) fn options(radius: f32) -> ArchipelagoOptions<YUp> {
    ArchipelagoOptions::<YUp>::from_agent_radius(radius)
}

/// Sample distances for point queries: `horizontal` and `vertical` in metres.
pub(crate) fn sample_distance(horizontal: f32, vertical: f32) -> PointSampleDistance3d {
    PointSampleDistance3d {
        horizontal_distance: horizontal,
        distance_above: vertical,
        distance_below: vertical,
        vertical_preference_ratio: 2.0,
        animation_link_max_vertical_distance: vertical,
    }
}

/// Builds a validated landmass mesh from a tile, leaving out polygons for which `skip` is true.
///
/// Returns `Ok(None)` when no polygon is left. The polygon type index is the area id.
pub(crate) fn tile_mesh(
    tile: &NavTile,
    skip: &dyn Fn(&NavPolygon) -> bool,
) -> Result<Option<ValidNavigationMesh<YUp>>, NavError> {
    let mut polygons = Vec::new();
    let mut type_indices = Vec::new();
    for polygon in &tile.polygons {
        if skip(polygon) {
            continue;
        }
        polygons.push(polygon.indices.iter().map(|&i| usize::from(i)).collect());
        type_indices.push(usize::from(polygon.area.0));
    }
    if polygons.is_empty() {
        return Ok(None);
    }
    let mesh = NavigationMesh::<YUp> {
        vertices: tile
            .vertices
            .iter()
            .map(|v| LmVec3::from_array(*v))
            .collect(),
        polygons,
        polygon_type_indices: type_indices,
        height_mesh: None,
    };
    mesh.validate()
        .map(Some)
        .map_err(|e| NavError::Backend(e.to_string()))
}

/// An agent at `position` (engine coordinates) with the given radius and speed.
pub(crate) fn agent(position: Vec3, radius: f32, speed: f32) -> Agent<YUp> {
    Agent::create(to_lm(position), LmVec3::ZERO, radius, speed, speed)
}

/// An island placed at the origin with the given mesh.
pub(crate) fn island(mesh: ValidNavigationMesh<YUp>) -> Island<YUp> {
    Island::new(
        Transform {
            translation: LmVec3::ZERO,
            rotation: 0.0,
        },
        Arc::new(mesh),
    )
}

/// An off-mesh link between two edges, in backend coordinates.
pub(crate) fn link(
    start_edge: (LmVec3, LmVec3),
    end_edge: (LmVec3, LmVec3),
    kind: usize,
    cost: f32,
    bidirectional: bool,
) -> AnimationLink<YUp> {
    AnimationLink {
        start_edge,
        end_edge,
        kind,
        cost,
        bidirectional,
    }
}

/// Off-mesh link handle as the backend knows it.
pub(crate) type LinkHandle = AnimationLinkId;

/// Link kinds an agent may use, as the backend filter.
pub(crate) fn permitted(kinds: HashSet<usize>) -> PermittedAnimationLinks {
    PermittedAnimationLinks::Kinds(Arc::new(kinds))
}

/// Shortest path between two sampled points, as engine points (links give start and end).
pub(crate) fn find_path(
    arch: &Arch,
    start: &SampledPoint<'_, YUp>,
    end: &SampledPoint<'_, YUp>,
    permitted: PermittedAnimationLinks,
) -> Result<Vec<Vec3>, NavError> {
    let steps = arch
        .find_path(start, end, &HashMap::new(), permitted)
        .map_err(|e| NavError::Backend(e.to_string()))?;
    let mut points = Vec::new();
    for step in steps {
        match step {
            PathStep::Waypoint(p) => points.push(from_lm(p)),
            PathStep::AnimationLink {
                start_point,
                end_point,
                ..
            } => {
                points.push(from_lm(start_point));
                points.push(from_lm(end_point));
            }
        }
    }
    Ok(points)
}

/// Agent handle as the backend knows it.
pub(crate) type AgentHandle = AgentId;

/// Engine-level status of an agent from the backend state.
pub(crate) fn status_of(agent: &Agent<YUp>) -> AgentStatus {
    match agent.state() {
        AgentState::Idle | AgentState::Paused => AgentStatus::Idle,
        AgentState::ReachedTarget => AgentStatus::Reached,
        AgentState::Moving | AgentState::ReachedAnimationLink | AgentState::UsingAnimationLink => {
            AgentStatus::Moving
        }
        AgentState::NoPath => AgentStatus::NoPath,
        AgentState::AgentNotOnNavMesh | AgentState::TargetNotOnNavMesh => AgentStatus::OffMesh,
    }
}

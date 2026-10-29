//! [`NavWorld`]: the navigation mesh of a level as one landmass archipelago.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Display;
use std::sync::Arc;

use landmass::{Archipelago, IslandId};
use redox_math::Vec3;

use crate::backend::{self, Arch};
use crate::blockers::{Blocker, NavBlockerId};
use crate::data::{AgentParams, NavAreaId, NavData, NavPolygon, NavTile};
use crate::error::NavError;
use crate::links::{LinkEntry, NavLinkId};

/// Class of agent a path is planned for. Each class sees the links whose class mask has its bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AgentClass(pub u8);

/// The navigation data of a level, ready for queries.
///
/// Every tile becomes one island. Changing one tile replaces only that island, so agents on
/// other tiles keep their paths.
pub struct NavWorld {
    pub(crate) archipelago: Arch,
    pub(crate) agent: AgentParams,
    pub(crate) tile_size: f32,
    pub(crate) tiles: BTreeMap<(i32, i32), NavTile>,
    islands: HashMap<(i32, i32), IslandId>,
    pub(crate) blocked: HashSet<NavAreaId>,
    pub(crate) blockers: BTreeMap<NavBlockerId, Blocker>,
    pub(crate) next_blocker: u32,
    pub(crate) links: HashMap<NavLinkId, LinkEntry>,
    pub(crate) next_link: u32,
}

impl NavWorld {
    /// Builds a world from baked data. Every tile becomes one island. Area costs default to 1.0.
    ///
    /// # Errors
    ///
    /// [`NavError::Backend`] when a tile does not form a valid mesh.
    pub fn from_data(data: &NavData) -> Result<Self, NavError> {
        let mut world = Self {
            archipelago: Archipelago::new(backend::options(data.agent.radius)),
            agent: data.agent,
            tile_size: data.tile_size,
            tiles: BTreeMap::new(),
            islands: HashMap::new(),
            blocked: HashSet::new(),
            blockers: BTreeMap::new(),
            next_blocker: 0,
            links: HashMap::new(),
            next_link: 0,
        };
        for tile in &data.tiles {
            world.tiles.insert(tile.coord, tile.clone());
        }
        let mut areas: Vec<u8> = data
            .tiles
            .iter()
            .flat_map(|tile| tile.polygons.iter().map(|polygon| polygon.area.0))
            .collect();
        areas.sort_unstable();
        areas.dedup();
        for area in areas {
            world
                .archipelago
                .set_type_index_cost(usize::from(area), 1.0)
                .map_err(backend_error)?;
        }
        world.rebuild_all();
        world.refresh();
        Ok(world)
    }

    /// Agent parameters the mesh was baked for.
    pub fn agent(&self) -> AgentParams {
        self.agent
    }

    /// Tile edge length in metres.
    pub fn tile_size(&self) -> f32 {
        self.tile_size
    }

    /// Sets the cost multiplier of an area (1.0 is the default; above 1.0 is more expensive).
    /// `f32::INFINITY` removes the area's polygons from the mesh.
    ///
    /// # Errors
    ///
    /// [`NavError::BadCost`] for NaN, zero or negative costs.
    pub fn set_area_cost(&mut self, area: NavAreaId, cost: f32) -> Result<(), NavError> {
        if cost.is_nan() || cost <= 0.0 {
            return Err(NavError::BadCost);
        }
        let changed = if cost.is_infinite() {
            self.blocked.insert(area)
        } else {
            self.archipelago
                .set_type_index_cost(usize::from(area.0), cost)
                .map_err(backend_error)?;
            self.blocked.remove(&area)
        };
        if changed {
            self.rebuild_all();
        }
        self.refresh();
        Ok(())
    }

    /// Replaces the tile at `tile.coord`. Other tiles are not touched.
    ///
    /// # Errors
    ///
    /// [`NavError::Backend`] when the new tile is not a valid mesh; the world is left unchanged then.
    pub fn replace_tile(&mut self, tile: NavTile) -> Result<(), NavError> {
        backend::tile_mesh(&tile, &|polygon| self.skip_polygon(&tile, polygon))?;
        let coord = tile.coord;
        self.tiles.insert(coord, tile);
        self.install_tile(coord)?;
        self.refresh();
        Ok(())
    }

    /// Removes the tile at `coord`; returns whether it existed.
    pub fn remove_tile(&mut self, coord: (i32, i32)) -> bool {
        let existed = self.tiles.remove(&coord).is_some();
        self.remove_island_of(coord);
        self.refresh();
        existed
    }

    /// Brings the archipelago to the state of `tiles`: one island per tile that has polygons.
    ///
    /// A tile that does not form a valid mesh is logged and left without an island; tiles are
    /// validated when they are added, so this only happens for data changed by hand.
    pub(crate) fn rebuild_all(&mut self) {
        let coords: Vec<(i32, i32)> = self.tiles.keys().copied().collect();
        for coord in coords {
            if let Err(error) = self.install_tile(coord) {
                log::warn!("navmesh tile {coord:?} has no island: {error}");
            }
        }
    }

    /// Creates, replaces or removes the island of one tile, following `tiles`, areas and blockers.
    fn install_tile(&mut self, coord: (i32, i32)) -> Result<(), NavError> {
        let mesh = match self.tiles.get(&coord) {
            Some(tile) => backend::tile_mesh(tile, &|polygon| self.skip_polygon(tile, polygon))?,
            None => None,
        };
        let existing = self.islands.get(&coord).copied();
        match (mesh, existing) {
            (Some(mesh), Some(id)) => {
                if let Some(mut island) = self.archipelago.get_island_mut(id) {
                    island.set_nav_mesh(Arc::new(mesh));
                }
            }
            (Some(mesh), None) => {
                let id = self.archipelago.add_island(backend::island(mesh));
                self.islands.insert(coord, id);
            }
            (None, _) => self.remove_island_of(coord),
        }
        Ok(())
    }

    /// Removes the island of a tile, if it has one.
    fn remove_island_of(&mut self, coord: (i32, i32)) {
        if let Some(id) = self.islands.remove(&coord) {
            if self.archipelago.get_island(id).is_some() {
                self.archipelago.remove_island(id);
            }
        }
    }

    /// Whether a polygon is left out of the mesh: blocked area or covered by a blocker.
    fn skip_polygon(&self, tile: &NavTile, polygon: &NavPolygon) -> bool {
        self.blocked.contains(&polygon.area) || self.blocker_covers(tile, polygon)
    }

    /// Advances the archipelago by zero time: applies pending island changes so queries work.
    pub(crate) fn refresh(&mut self) {
        self.archipelago.update(0.0);
    }

    /// Edges of every polygon of every stored tile, for debug drawing.
    pub fn polygon_edges(&self) -> Vec<[Vec3; 2]> {
        let mut edges = Vec::new();
        for tile in self.tiles.values() {
            for polygon in &tile.polygons {
                let corners: Vec<[f32; 3]> = polygon
                    .indices
                    .iter()
                    .filter_map(|&i| tile.vertices.get(usize::from(i)).copied())
                    .collect();
                for (index, corner) in corners.iter().enumerate() {
                    let next = corners[(index + 1) % corners.len()];
                    edges.push([Vec3::from_array(*corner), Vec3::from_array(next)]);
                }
            }
        }
        edges
    }
}

/// Maps a backend error to [`NavError::Backend`].
pub(crate) fn backend_error<E: Display>(error: E) -> NavError {
    NavError::Backend(error.to_string())
}

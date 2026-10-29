//! Plain data of a baked navigation mesh: no backend types appear here.

/// Parameters of the agent the mesh is built for. Units: metres and degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AgentParams {
    /// Agent radius; the mesh keeps this distance from obstacles.
    pub radius: f32,
    /// Agent height; overhangs lower than this are not walkable.
    pub height: f32,
    /// Highest ledge the agent can step onto.
    pub max_step: f32,
    /// Steepest walkable slope in degrees.
    pub max_slope_deg: f32,
}

impl Default for AgentParams {
    fn default() -> Self {
        Self {
            radius: 0.4,
            height: 1.8,
            max_step: 0.4,
            max_slope_deg: 45.0,
        }
    }
}

/// Area type of a polygon. Costs are set when the world is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NavAreaId(pub u8);

impl NavAreaId {
    /// Ordinary walkable ground.
    pub const WALKABLE: Self = Self(0);
    /// Not walkable: no polygons are produced for it.
    pub const NOT_WALKABLE: Self = Self(255);
}

/// One baked tile: convex polygons over shared vertices, in world space.
#[derive(Clone, Debug, PartialEq)]
pub struct NavTile {
    /// Tile grid coordinate (x, z) in units of the tile size.
    pub coord: (i32, i32),
    /// World-space vertices (x, y up, z).
    pub vertices: Vec<[f32; 3]>,
    /// Convex polygons referring to `vertices`.
    pub polygons: Vec<NavPolygon>,
}

/// A convex polygon of a tile.
#[derive(Clone, Debug, PartialEq)]
pub struct NavPolygon {
    /// Indices into [`NavTile::vertices`]: 3 to 6 of them, counter-clockwise seen from above (+Y).
    pub indices: Vec<u16>,
    /// Area type of the polygon.
    pub area: NavAreaId,
}

/// All tiles of one level for one agent class.
#[derive(Clone, Debug, PartialEq)]
pub struct NavData {
    /// The agent the mesh was built for.
    pub agent: AgentParams,
    /// Edge length of a tile in metres.
    pub tile_size: f32,
    /// Baked tiles; empty tiles are not stored.
    pub tiles: Vec<NavTile>,
}

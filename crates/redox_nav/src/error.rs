//! Error types of the navigation crate.

use thiserror::Error;

/// Errors when reading the `.nav` format.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum NavFormatError {
    /// The file does not start with the `RNAV` magic.
    #[error("not a .nav file: bad magic")]
    BadMagic,
    /// The file was written by a format version this build does not know.
    #[error("unknown .nav format version {0}")]
    UnknownVersion(u32),
    /// The data ends before the header or a tile is complete.
    #[error("truncated .nav data")]
    Truncated,
    /// A polygon has fewer than 3 or more than 6 vertex indices.
    #[error("polygon with {0} indices (must be 3 to 6)")]
    BadPolygonSize(usize),
    /// A polygon refers to a vertex that the tile does not have.
    #[error("tile {tile}: vertex index {index} is out of range ({vertices} vertices)")]
    IndexOutOfRange {
        /// Index of the tile in the file.
        tile: usize,
        /// The offending vertex index.
        index: u16,
        /// Number of vertices of the tile.
        vertices: usize,
    },
}

/// Errors of the navigation world.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum NavError {
    /// An area cost is NaN, zero or negative. Positive infinity is allowed and blocks the area.
    #[error("area cost must be a positive number, or infinity to block the area")]
    BadCost,
    /// An end of a link is further than 0.5 m from the navigation mesh.
    #[error("a link end is more than 0.5 m away from the navigation mesh")]
    LinkOffMesh,
    /// A blocker footprint has fewer than 3 corners, a non-finite value, or `min_y > max_y`.
    #[error("a blocker needs a footprint of at least 3 finite corners and min_y <= max_y")]
    BadBlocker,
    /// An agent radius or speed is not positive and finite.
    #[error("agent radius and speed must be positive and finite")]
    BadAgent,
    /// The navigation backend rejected the data.
    #[error("navigation backend failed: {0}")]
    Backend(String),
}

/// Errors when baking navigation data from geometry.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BakeError {
    /// The geometry has no triangles.
    #[error("the geometry has no triangles")]
    EmptyGeometry,
    /// The tile size is not a positive finite number.
    #[error("the tile size must be a positive finite number of metres")]
    BadTileSize,
    /// The geometry arrays do not agree (area count, index range).
    #[error("bad geometry: {0}")]
    BadGeometry(&'static str),
    /// The voxel backend (rerecast) failed.
    #[error("navmesh backend failed: {0}")]
    Backend(String),
}

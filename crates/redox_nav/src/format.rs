//! The cooked `.nav` format (little endian, version 1).
//!
//! Layout: magic `RNAV`, `u32` version, four `f32` agent parameters (radius, height,
//! max step, max slope), `f32` tile size, `u32` tile count, then per tile: `i32 x`, `i32 y`,
//! `u32 vertex count`, `u32 polygon count`, the vertices as `3 x f32`, and per polygon:
//! `u8 area`, `u8 index count`, the indices as `u16`.

use crate::data::{AgentParams, NavAreaId, NavData, NavPolygon, NavTile};
use crate::error::NavFormatError;

const MAGIC: [u8; 4] = *b"RNAV";
const VERSION: u32 = 1;
const MIN_POLYGON_INDICES: usize = 3;
const MAX_POLYGON_INDICES: usize = 6;

impl NavData {
    /// Serialises to the `.nav` format. Deterministic: tiles are written sorted by `coord`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut tiles: Vec<&NavTile> = self.tiles.iter().collect();
        tiles.sort_by_key(|tile| tile.coord);

        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        push_u32(&mut out, VERSION);
        push_f32(&mut out, self.agent.radius);
        push_f32(&mut out, self.agent.height);
        push_f32(&mut out, self.agent.max_step);
        push_f32(&mut out, self.agent.max_slope_deg);
        push_f32(&mut out, self.tile_size);
        push_u32(&mut out, tiles.len() as u32);
        for tile in tiles {
            push_i32(&mut out, tile.coord.0);
            push_i32(&mut out, tile.coord.1);
            push_u32(&mut out, tile.vertices.len() as u32);
            push_u32(&mut out, tile.polygons.len() as u32);
            for vertex in &tile.vertices {
                for component in vertex {
                    push_f32(&mut out, *component);
                }
            }
            for polygon in &tile.polygons {
                out.push(polygon.area.0);
                out.push(polygon.indices.len() as u8);
                for index in &polygon.indices {
                    out.extend_from_slice(&index.to_le_bytes());
                }
            }
        }
        out
    }

    /// Parses `.nav` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`NavFormatError`] for a wrong magic or version, truncated data, a polygon
    /// with fewer than 3 or more than 6 indices, or an index outside its tile.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavFormatError> {
        let mut reader = Reader {
            data: bytes,
            pos: 0,
        };
        if reader.take::<4>()? != MAGIC {
            return Err(NavFormatError::BadMagic);
        }
        let version = reader.u32()?;
        if version != VERSION {
            return Err(NavFormatError::UnknownVersion(version));
        }
        let agent = AgentParams {
            radius: reader.f32()?,
            height: reader.f32()?,
            max_step: reader.f32()?,
            max_slope_deg: reader.f32()?,
        };
        let tile_size = reader.f32()?;
        let tile_count = reader.u32()? as usize;
        let mut tiles = Vec::new();
        for tile_index in 0..tile_count {
            tiles.push(read_tile(&mut reader, tile_index)?);
        }
        Ok(Self {
            agent,
            tile_size,
            tiles,
        })
    }
}

fn read_tile(reader: &mut Reader<'_>, tile_index: usize) -> Result<NavTile, NavFormatError> {
    let x = reader.i32()?;
    let y = reader.i32()?;
    let vertex_count = reader.u32()? as usize;
    let polygon_count = reader.u32()? as usize;
    let mut vertices = Vec::new();
    for _ in 0..vertex_count {
        vertices.push([reader.f32()?, reader.f32()?, reader.f32()?]);
    }
    let mut polygons = Vec::new();
    for _ in 0..polygon_count {
        let area = NavAreaId(reader.u8()?);
        let index_count = reader.u8()? as usize;
        if !(MIN_POLYGON_INDICES..=MAX_POLYGON_INDICES).contains(&index_count) {
            return Err(NavFormatError::BadPolygonSize(index_count));
        }
        let mut indices = Vec::with_capacity(index_count);
        for _ in 0..index_count {
            let index = reader.u16()?;
            if usize::from(index) >= vertex_count {
                return Err(NavFormatError::IndexOutOfRange {
                    tile: tile_index,
                    index,
                    vertices: vertex_count,
                });
            }
            indices.push(index);
        }
        polygons.push(NavPolygon { indices, area });
    }
    Ok(NavTile {
        coord: (x, y),
        vertices,
        polygons,
    })
}

/// Bounds-checked cursor over the input bytes.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], NavFormatError> {
        let end = self.pos.checked_add(N).ok_or(NavFormatError::Truncated)?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(NavFormatError::Truncated)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        self.pos = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, NavFormatError> {
        Ok(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, NavFormatError> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    fn u32(&mut self) -> Result<u32, NavFormatError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn i32(&mut self) -> Result<i32, NavFormatError> {
        Ok(i32::from_le_bytes(self.take()?))
    }

    fn f32(&mut self) -> Result<f32, NavFormatError> {
        Ok(f32::from_le_bytes(self.take()?))
    }
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

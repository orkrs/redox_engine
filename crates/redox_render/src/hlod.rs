//! Hierarchical LOD (HLOD): one simplified proxy mesh per world cell, and a
//! pure state machine that cross-fades between the detail objects of a cell
//! and that proxy by distance.
//!
//! Three parts, all CPU-only:
//! - [`build_proxy`] merges, welds, simplifies and colours a group of meshes.
//! - [`HlodProxy::to_bytes`] / [`HlodProxy::from_bytes`] read and write the
//!   `.hlod` file format (one file per cell).
//! - [`step_cell_lod`] / [`opacities`] decide what the viewer sees for a cell.

use std::collections::HashMap;

use meshopt::{
    SimplifyOptions, VertexDataAdapter, simplify_scale, simplify_with_attributes_and_locks,
};
use redox_math::{Mat4, Vec3};
use thiserror::Error;

/// Bytes per stored vertex position or normal (three `f32`).
const VEC3_BYTES: usize = 12;
/// Bytes per stored vertex colour (RGB8).
const COLOUR_BYTES: usize = 3;
/// Bytes per stored triangle index.
const INDEX_BYTES: usize = 4;
/// Size of the `.hlod` header: magic, version, cell, source triangles, counts, AABB.
const HEADER_BYTES: usize = 4 + 4 + 4 + 4 + 4 + 4 + 4 + 2 * VEC3_BYTES;
/// Magic bytes at the start of every `.hlod` file.
const MAGIC: [u8; 4] = *b"RHLD";
/// Current `.hlod` format version.
const HLOD_VERSION: u32 = 1;
/// Weight of the vertex colour in the simplification error metric.
const COLOUR_ATTRIBUTE_WEIGHT: f32 = 0.5;

/// Errors of HLOD building and of decoding `.hlod` files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HlodError {
    /// No input meshes were given.
    #[error("no input meshes for the HLOD proxy")]
    NoInput,
    /// The inputs have no usable triangles, have invalid indices or non-finite
    /// positions, or the parameters are invalid.
    #[error("degenerate HLOD input")]
    Degenerate,
    /// The bytes do not start with the `.hlod` magic.
    #[error("not an .hlod file (bad magic)")]
    BadMagic,
    /// The file was written by a different format version.
    #[error("unsupported .hlod version {0}")]
    UnsupportedVersion(u32),
    /// The file ends before the data its header promises.
    #[error("truncated .hlod file")]
    Truncated,
    /// The file has trailing bytes or indices outside the vertex range.
    #[error("corrupt .hlod file")]
    Corrupt,
}

/// A mesh to merge: world-space triangles with one flat colour.
#[derive(Debug, Clone, Copy)]
pub struct HlodInput<'a> {
    /// Vertex positions in the mesh's local space.
    pub positions: &'a [[f32; 3]],
    /// Triangle list indexing `positions` (three indices per triangle).
    pub indices: &'a [u32],
    /// Local-to-world transform applied to `positions`.
    pub transform: Mat4,
    /// Flat linear RGB colour of the source material, each channel in `0..=1`.
    pub color: [f32; 3],
}

/// Parameters of [`build_proxy`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HlodParams {
    /// Upper bound on the number of triangles of the proxy (e.g. 2000).
    pub target_triangles: usize,
    /// Largest allowed simplification error, in world units (e.g. 0.05 of the
    /// cell diagonal). Simplification stops early when this is reached.
    pub max_error: f32,
    /// Vertices whose world positions fall into the same cube of this size
    /// (world units) are merged (e.g. 1e-4).
    pub weld_epsilon: f32,
}

/// A simplified proxy mesh in world space.
#[derive(Debug, Clone, PartialEq)]
pub struct HlodProxy {
    /// Vertex positions in world space.
    pub vertices: Vec<[f32; 3]>,
    /// Unit vertex normals, smoothed from the face normals of the proxy.
    pub normals: Vec<[f32; 3]>,
    /// Linear RGB vertex colours, each channel a multiple of 1/255.
    pub colors: Vec<[f32; 3]>,
    /// Triangle list indexing `vertices`.
    pub indices: Vec<u32>,
    /// Number of triangles of all inputs before simplification.
    pub source_triangles: usize,
}

/// Vertices merged across all inputs, with area-weighted vertex colours.
struct Welded {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 3]>,
    indices: Vec<u32>,
    source_triangles: usize,
}

/// Accumulates inputs into one welded vertex pool.
struct Welder {
    inverse_epsilon: f32,
    lookup: HashMap<[i64; 3], u32>,
    positions: Vec<Vec3>,
    colour_sum: Vec<Vec3>,
    area_sum: Vec<f32>,
    indices: Vec<u32>,
    source_triangles: usize,
}

impl Welder {
    /// A welder that merges positions closer than `epsilon`.
    fn new(epsilon: f32) -> Result<Self, HlodError> {
        if !(epsilon.is_finite() && epsilon > 0.0) {
            return Err(HlodError::Degenerate);
        }
        Ok(Self {
            inverse_epsilon: 1.0 / epsilon,
            lookup: HashMap::new(),
            positions: Vec::new(),
            colour_sum: Vec::new(),
            area_sum: Vec::new(),
            indices: Vec::new(),
            source_triangles: 0,
        })
    }

    /// Id of the welded vertex at `position`, creating it when new.
    fn vertex(&mut self, position: Vec3) -> Result<u32, HlodError> {
        let scale = self.inverse_epsilon;
        let key = [
            (position.x * scale).round() as i64,
            (position.y * scale).round() as i64,
            (position.z * scale).round() as i64,
        ];
        if let Some(&id) = self.lookup.get(&key) {
            return Ok(id);
        }
        let id = to_u32(self.positions.len())?;
        self.positions.push(position);
        self.colour_sum.push(Vec3::ZERO);
        self.area_sum.push(0.0);
        self.lookup.insert(key, id);
        Ok(id)
    }

    /// Transforms one input to world space and appends its triangles.
    fn add_input(&mut self, input: &HlodInput<'_>) -> Result<(), HlodError> {
        if input.indices.len() % 3 != 0 {
            return Err(HlodError::Degenerate);
        }
        let world: Vec<Vec3> = input
            .positions
            .iter()
            .map(|p| input.transform.transform_point3(Vec3::from_array(*p)))
            .collect();
        if world.iter().any(|p| !p.is_finite()) {
            return Err(HlodError::Degenerate);
        }
        let local_to_welded: Vec<u32> = world
            .iter()
            .map(|p| self.vertex(*p))
            .collect::<Result<_, _>>()?;
        let colour = Vec3::from_array(input.color);
        for tri in input.indices.chunks_exact(3) {
            let (Some(&a), Some(&b), Some(&c)) = (
                world.get(tri[0] as usize),
                world.get(tri[1] as usize),
                world.get(tri[2] as usize),
            ) else {
                return Err(HlodError::Degenerate);
            };
            let area = 0.5 * (b - a).cross(c - a).length();
            let weight = area.max(f32::MIN_POSITIVE);
            for &local in tri {
                let id = local_to_welded
                    .get(local as usize)
                    .copied()
                    .ok_or(HlodError::Degenerate)?;
                self.colour_sum[id as usize] += colour * weight;
                self.area_sum[id as usize] += weight;
                self.indices.push(id);
            }
            self.source_triangles += 1;
        }
        Ok(())
    }

    /// Finishes welding: vertex colours become area-weighted means.
    fn finish(self) -> Welded {
        let colors = self
            .colour_sum
            .iter()
            .zip(&self.area_sum)
            .map(|(sum, area)| {
                if *area > 0.0 {
                    (*sum / *area).to_array()
                } else {
                    [0.0; 3]
                }
            })
            .collect();
        Welded {
            positions: self.positions.iter().map(|p| p.to_array()).collect(),
            colors,
            indices: self.indices,
            source_triangles: self.source_triangles,
        }
    }
}

/// Merges the inputs into one mesh, welds, simplifies to at most
/// `target_triangles` (stopping early at `max_error`), and colours every
/// vertex with the area-weighted mean colour of the source triangles around it.
///
/// Normals are the area-weighted sums of the face normals of the simplified
/// proxy, normalised per welded vertex. Colours are rounded to 1/255 so that
/// the `.hlod` encoding is lossless.
///
/// # Errors
/// [`HlodError::NoInput`] for an empty slice; [`HlodError::Degenerate`] for
/// invalid indices, non-finite positions, invalid parameters, or inputs with
/// no triangles.
pub fn build_proxy(inputs: &[HlodInput<'_>], params: &HlodParams) -> Result<HlodProxy, HlodError> {
    if inputs.is_empty() {
        return Err(HlodError::NoInput);
    }
    if !(params.max_error.is_finite() && params.max_error >= 0.0) {
        return Err(HlodError::Degenerate);
    }
    let mut welder = Welder::new(params.weld_epsilon)?;
    for input in inputs {
        welder.add_input(input)?;
    }
    let welded = welder.finish();
    if welded.indices.is_empty() {
        return Err(HlodError::Degenerate);
    }
    let source_triangles = welded.source_triangles;
    let simplified = simplify_welded(&welded, params)?;
    let (vertices, colors, indices) = compact(&welded, &simplified)?;
    if indices.is_empty() {
        return Err(HlodError::Degenerate);
    }
    let normals = vertex_normals(&vertices, &indices);
    Ok(HlodProxy {
        vertices,
        normals,
        colors: colors.into_iter().map(quantize_colour).collect(),
        indices,
        source_triangles,
    })
}

/// Index buffer after quadric-error simplification with colour as an attribute.
/// Meshes at or below the target are returned unchanged.
fn simplify_welded(welded: &Welded, params: &HlodParams) -> Result<Vec<u32>, HlodError> {
    let target = params.target_triangles.max(1);
    if welded.indices.len() / 3 <= target {
        return Ok(welded.indices.clone());
    }
    let adapter = position_adapter(&welded.positions)?;
    // meshopt expresses the error relative to the mesh extent.
    let extent = simplify_scale(&adapter).max(f32::MIN_POSITIVE);
    let relative_error = (params.max_error / extent).clamp(0.0, 1.0);
    let attributes: Vec<f32> = welded.colors.iter().flat_map(|c| *c).collect();
    let locks = vec![false; welded.positions.len()];
    Ok(simplify_with_attributes_and_locks(
        &welded.indices,
        &adapter,
        &attributes,
        &[COLOUR_ATTRIBUTE_WEIGHT; 3],
        COLOUR_ATTRIBUTE_WEIGHT_STRIDE,
        &locks,
        target * 3,
        relative_error,
        SimplifyOptions::LockBorder,
        None,
    ))
}

/// Bytes per vertex attribute block (meshopt wants the stride in bytes): three colour channels.
const COLOUR_ATTRIBUTE_WEIGHT_STRIDE: usize = 3 * std::mem::size_of::<f32>();

/// Meshopt view over a position array (three `f32` per vertex, offset 0).
fn position_adapter(positions: &[[f32; 3]]) -> Result<VertexDataAdapter<'_>, HlodError> {
    VertexDataAdapter::new(bytemuck::cast_slice(positions), VEC3_BYTES, 0)
        .map_err(|_| HlodError::Degenerate)
}

/// Keeps only the vertices referenced by `indices` and renumbers them in order
/// of first use.
fn compact(
    welded: &Welded,
    indices: &[u32],
) -> Result<(Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>), HlodError> {
    let mut remap = vec![u32::MAX; welded.positions.len()];
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    let mut compacted = Vec::with_capacity(indices.len());
    for &old in indices {
        let old_index = old as usize;
        let current = *remap.get(old_index).ok_or(HlodError::Degenerate)?;
        let new = if current == u32::MAX {
            let id = to_u32(positions.len())?;
            positions.push(
                *welded
                    .positions
                    .get(old_index)
                    .ok_or(HlodError::Degenerate)?,
            );
            colors.push(*welded.colors.get(old_index).ok_or(HlodError::Degenerate)?);
            remap[old_index] = id;
            id
        } else {
            current
        };
        compacted.push(new);
    }
    Ok((positions, colors, compacted))
}

/// Area-weighted face normals, summed per vertex and normalised. Vertices
/// without a usable face get `+Y`.
fn vertex_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let position = |i: u32| {
        positions
            .get(i as usize)
            .map_or(Vec3::ZERO, |p| Vec3::from_array(*p))
    };
    let mut sums = vec![Vec3::ZERO; positions.len()];
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        let normal = (position(b) - position(a)).cross(position(c) - position(a));
        for corner in [a, b, c] {
            if let Some(sum) = sums.get_mut(corner as usize) {
                *sum += normal;
            }
        }
    }
    sums.into_iter()
        .map(|sum| {
            let unit = sum.normalize_or_zero();
            if unit == Vec3::ZERO {
                Vec3::Y.to_array()
            } else {
                unit.to_array()
            }
        })
        .collect()
}

/// Rounds each channel to the nearest 1/255 step.
fn quantize_colour(colour: [f32; 3]) -> [f32; 3] {
    colour.map(|c| colour_byte(c) as f32 / 255.0)
}

/// Encodes one linear channel as a byte.
fn colour_byte(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn to_u32(len: usize) -> Result<u32, HlodError> {
    u32::try_from(len).map_err(|_| HlodError::Degenerate)
}

impl HlodProxy {
    /// Axis-aligned bounds of the vertices (zeros when there are none).
    fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let Some(first) = self.vertices.first() else {
            return ([0.0; 3], [0.0; 3]);
        };
        self.vertices.iter().fold((*first, *first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])],
                [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])],
            )
        })
    }

    /// Encodes the proxy of `cell` as an `.hlod` file.
    ///
    /// Layout (little endian): magic `RHLD`, version `u32`, cell x and y `i32`,
    /// source triangle count `u32`, vertex count `u32`, index count `u32`,
    /// AABB min and max (`f32` x 6), then positions, normals (`f32` x 3 each),
    /// colours (RGB8) and indices (`u32`). The output is deterministic.
    pub fn to_bytes(&self, cell: (i32, i32)) -> Vec<u8> {
        let (min, max) = self.bounds();
        let mut out = Vec::with_capacity(
            HEADER_BYTES
                + self.vertices.len() * (2 * VEC3_BYTES + COLOUR_BYTES)
                + self.indices.len() * INDEX_BYTES,
        );
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&HLOD_VERSION.to_le_bytes());
        out.extend_from_slice(&cell.0.to_le_bytes());
        out.extend_from_slice(&cell.1.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.source_triangles)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(
            &u32::try_from(self.vertices.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(
            &u32::try_from(self.indices.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for value in min.iter().chain(max.iter()) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for vector in self.vertices.iter().chain(self.normals.iter()) {
            for value in vector {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for colour in &self.colors {
            out.extend(colour.iter().map(|c| colour_byte(*c)));
        }
        for index in &self.indices {
            out.extend_from_slice(&index.to_le_bytes());
        }
        out
    }

    /// Decodes an `.hlod` file written by [`HlodProxy::to_bytes`], returning the
    /// cell key and the proxy. The stored AABB is not re-checked; it is derived
    /// from the vertices when needed.
    ///
    /// # Errors
    /// [`HlodError::BadMagic`], [`HlodError::UnsupportedVersion`],
    /// [`HlodError::Truncated`] when the data is shorter than the header
    /// promises, and [`HlodError::Corrupt`] for trailing bytes or out-of-range
    /// indices.
    pub fn from_bytes(bytes: &[u8]) -> Result<((i32, i32), Self), HlodError> {
        let mut reader = Reader { bytes };
        if reader.take(MAGIC.len())? != &MAGIC[..] {
            return Err(HlodError::BadMagic);
        }
        let version = reader.u32()?;
        if version != HLOD_VERSION {
            return Err(HlodError::UnsupportedVersion(version));
        }
        let cell = (reader.i32()?, reader.i32()?);
        let source_triangles = reader.u32()? as usize;
        let vertex_count = reader.u32()? as usize;
        let index_count = reader.u32()? as usize;
        reader.take(2 * VEC3_BYTES)?;

        let body = vertex_count
            .checked_mul(2 * VEC3_BYTES + COLOUR_BYTES)
            .and_then(|v| index_count.checked_mul(INDEX_BYTES)?.checked_add(v))
            .ok_or(HlodError::Corrupt)?;
        match reader.bytes.len().cmp(&body) {
            std::cmp::Ordering::Less => return Err(HlodError::Truncated),
            std::cmp::Ordering::Greater => return Err(HlodError::Corrupt),
            std::cmp::Ordering::Equal => {}
        }

        let vertices = (0..vertex_count)
            .map(|_| reader.vec3())
            .collect::<Result<Vec<_>, _>>()?;
        let normals = (0..vertex_count)
            .map(|_| reader.vec3())
            .collect::<Result<Vec<_>, _>>()?;
        let colors = (0..vertex_count)
            .map(|_| reader.rgb8())
            .collect::<Result<Vec<_>, _>>()?;
        let indices = (0..index_count)
            .map(|_| reader.u32())
            .collect::<Result<Vec<_>, _>>()?;
        if indices.iter().any(|&i| i as usize >= vertex_count) {
            return Err(HlodError::Corrupt);
        }
        Ok((
            cell,
            Self {
                vertices,
                normals,
                colors,
                indices,
                source_triangles,
            },
        ))
    }
}

/// Sequential little-endian reader over a byte slice.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], HlodError> {
        let (head, tail) = self
            .bytes
            .split_at_checked(count)
            .ok_or(HlodError::Truncated)?;
        self.bytes = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], HlodError> {
        self.take(N)?.try_into().map_err(|_| HlodError::Truncated)
    }

    fn u32(&mut self) -> Result<u32, HlodError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, HlodError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, HlodError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn vec3(&mut self) -> Result<[f32; 3], HlodError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    fn rgb8(&mut self) -> Result<[f32; 3], HlodError> {
        let [r, g, b] = self.array::<COLOUR_BYTES>()?;
        Ok([r, g, b].map(|c| f32::from(c) / 255.0))
    }
}

/// What the viewer sees for one cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CellLod {
    /// The cell's own objects are drawn; the proxy is hidden.
    Detail,
    /// The proxy is drawn; the cell's objects are hidden.
    Proxy,
    /// Cross-fading. `to_proxy` tells the direction; `t` in `0..=1` is progress
    /// toward the target (the proxy when `to_proxy`, otherwise the detail).
    Fading {
        /// True when fading from the detail objects to the proxy.
        to_proxy: bool,
        /// Progress of the fade in `0..=1`.
        t: f32,
    },
}

/// Thresholds of the cell level-of-detail switch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HlodConfig {
    /// Distance from the viewer to the cell centre where the switch happens, in metres.
    pub switch_distance: f32,
    /// Half-width of the band around `switch_distance` that does not switch, in metres.
    pub hysteresis: f32,
    /// Duration of a full cross-fade, in seconds.
    pub fade_seconds: f32,
}

impl Default for HlodConfig {
    /// 120 m switch, 10 m hysteresis, 0.6 s fade.
    fn default() -> Self {
        Self {
            switch_distance: 120.0,
            hysteresis: 10.0,
            fade_seconds: 0.6,
        }
    }
}

/// Next state of a cell given the distance from the viewer to the cell centre
/// and the frame time `dt`.
///
/// Starting a fade needs the distance to leave the hysteresis band; a fade
/// that is running reverses from its current progress (`t` becomes `1 - t`) when
/// the distance crosses the opposite threshold, so the opacity does not jump.
/// NaN or negative `distance` or `dt` leave the state unchanged.
pub fn step_cell_lod(state: CellLod, distance: f32, dt: f32, cfg: &HlodConfig) -> CellLod {
    if distance.is_nan() || distance < 0.0 || dt.is_nan() || dt < 0.0 {
        return state;
    }
    let to_detail_below = cfg.switch_distance - cfg.hysteresis;
    let to_proxy_above = cfg.switch_distance + cfg.hysteresis;
    match state {
        CellLod::Detail => {
            if distance > to_proxy_above {
                CellLod::Fading {
                    to_proxy: true,
                    t: 0.0,
                }
            } else {
                CellLod::Detail
            }
        }
        CellLod::Proxy => {
            if distance < to_detail_below {
                CellLod::Fading {
                    to_proxy: false,
                    t: 0.0,
                }
            } else {
                CellLod::Proxy
            }
        }
        CellLod::Fading { to_proxy, t } => {
            let crossed_back = if to_proxy {
                distance < to_detail_below
            } else {
                distance > to_proxy_above
            };
            if crossed_back {
                return CellLod::Fading {
                    to_proxy: !to_proxy,
                    t: 1.0 - t.clamp(0.0, 1.0),
                };
            }
            let step = if cfg.fade_seconds > 0.0 {
                dt / cfg.fade_seconds
            } else {
                1.0
            };
            let next = t + step;
            match (next >= 1.0, to_proxy) {
                (true, true) => CellLod::Proxy,
                (true, false) => CellLod::Detail,
                (false, _) => CellLod::Fading { to_proxy, t: next },
            }
        }
    }
}

/// Opacities `(detail, proxy)` in `0..=1` for a state. They always sum to 1.
pub fn opacities(state: CellLod) -> (f32, f32) {
    match state {
        CellLod::Detail => (1.0, 0.0),
        CellLod::Proxy => (0.0, 1.0),
        CellLod::Fading { to_proxy, t } => {
            let t = t.clamp(0.0, 1.0);
            if to_proxy { (1.0 - t, t) } else { (t, 1.0 - t) }
        }
    }
}

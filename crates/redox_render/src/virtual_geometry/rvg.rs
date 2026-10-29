//! `.rvg` file format for paged cluster DAGs (VG-03 step 2).
//!
//! Layout (all integers and floats little-endian):
//!
//! ```text
//! header (fixed, 20 bytes):
//!   magic            [u8; 4]  = b"RVG1"
//!   version          u16      = 1
//!   resident_pages   u16      = pages 0..resident_pages are resident
//!   cluster_count   u32
//!   page_count      u32
//!   root_count      u32      (ClusterDag::root_count)
//! page table: page_count x { offset u64, len u32, hash u32 }
//! cluster_page:   cluster_count x u32
//! cluster records: cluster_count x DagCluster (72 bytes each)
//! pages: each page is self-contained (see `encode_page`)
//! ```
//!
//! The DAG itself is always resident. Pages hold only the vertices and the
//! indices of their clusters, so a page can be read and decoded on its own.

use std::collections::HashMap;

use super::dag::{ClusterDag, DagCluster, VGDagAsset};
use super::pages::{PagePlan, PageSpan};
use crate::mesh::Vertex;

const MAGIC: [u8; 4] = *b"RVG1";
const VERSION: u16 = 1;
/// Bytes of the fixed header before the root count.
const FIXED_HEADER_BYTES: usize = 16;
/// Fixed header plus the root count.
const HEADER_PREFIX_BYTES: usize = 20;
const PAGE_ENTRY_BYTES: usize = 16;
const CLUSTER_PAGE_BYTES: usize = 4;
const CLUSTER_RECORD_BYTES: usize = 72;
const PAGE_HEADER_BYTES: usize = 12;
const PAGE_CLUSTER_BYTES: usize = 12;
const VERTEX_BYTES: usize = 44;
const INDEX_BYTES: usize = 4;

/// Errors of reading or writing `.rvg` data.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RvgError {
    /// The first four bytes are not `RVG1`.
    #[error("not an RVG file (bad magic)")]
    BadMagic,
    /// The file version is not supported by this build.
    #[error("unsupported RVG version {0}")]
    UnsupportedVersion(u16),
    /// The data ends before the structure it describes.
    #[error("RVG data is truncated")]
    Truncated,
    /// The data is internally inconsistent.
    #[error("corrupt RVG data: {0}")]
    Corrupt(&'static str),
    /// The page bytes do not match the hash stored in the page table.
    #[error("page {page} failed its hash check")]
    HashMismatch {
        /// Page whose hash did not match.
        page: u32,
    },
    /// The requested page id is not in the page table.
    #[error("page {0} does not exist")]
    PageOutOfRange(u32),
    /// The page plan does not describe the given asset.
    #[error("page plan does not match the asset: {0}")]
    InvalidPlan(&'static str),
}

/// One entry of the page table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageEntry {
    /// Absolute byte offset of the page in the file.
    pub offset: u64,
    /// Length of the page in bytes.
    pub len: u32,
    /// FNV-1a hash of the page bytes.
    pub hash: u32,
}

/// Parsed header of an `.rvg` file: the DAG and the page table.
#[derive(Clone, Debug)]
pub struct RvgIndex {
    /// The full hierarchy, without geometry.
    pub dag: ClusterDag,
    /// Page id for every cluster.
    pub cluster_page: Vec<u32>,
    /// Page table, indexed by page id.
    pub entries: Vec<PageEntry>,
    /// Number of leading pages that are always resident.
    pub resident_page_count: u32,
    header_len: usize,
}

impl RvgIndex {
    /// Number of pages in the file.
    pub fn page_count(&self) -> usize {
        self.entries.len()
    }

    /// Size of the header in bytes; page data starts after it.
    pub fn header_len(&self) -> usize {
        self.header_len
    }

    /// Page table entry of `page`, if it exists.
    pub fn page_entry(&self, page: u32) -> Option<PageEntry> {
        self.entries.get(page as usize).copied()
    }

    /// True when `page` is one of the always-resident leading pages.
    pub fn is_resident(&self, page: u32) -> bool {
        page < self.resident_page_count
    }
}

/// Geometry of one cluster stored in a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageCluster {
    /// Cluster id in the DAG.
    pub cluster: u32,
    /// Offset of the cluster's first index in `PageData::indices`.
    pub index_offset: u32,
    /// Number of indices of the cluster.
    pub index_count: u32,
}

/// Decoded contents of one page. Indices refer to `vertices` of this page.
#[derive(Clone, Debug, Default)]
pub struct PageData {
    /// Clusters stored in the page, in file order.
    pub clusters: Vec<PageCluster>,
    /// Vertices used by the page's clusters.
    pub vertices: Vec<Vertex>,
    /// Triangle list indices local to the page.
    pub indices: Vec<u32>,
}

/// Serialises `asset` into `.rvg` bytes laid out according to `plan`.
///
/// `plan.resident_pages` must be `0..k`. Every page is self-contained: its
/// vertices are copied and its indices are remapped to them.
pub fn write_rvg(asset: &VGDagAsset, plan: &PagePlan) -> Result<Vec<u8>, RvgError> {
    validate_plan(asset, plan)?;
    let mut page_blobs = Vec::with_capacity(plan.pages.len());
    for span in &plan.pages {
        page_blobs.push(encode_page(asset, plan, *span)?);
    }
    let cluster_count = asset.dag.clusters.len();
    let header_len = header_len_from_counts(cluster_count, plan.pages.len())
        .ok_or(RvgError::InvalidPlan("header too large"))?;

    let mut entries = Vec::with_capacity(page_blobs.len());
    let mut offset = u64::try_from(header_len).map_err(|_| RvgError::InvalidPlan("too large"))?;
    for blob in &page_blobs {
        let len = u32::try_from(blob.len()).map_err(|_| RvgError::InvalidPlan("page too large"))?;
        entries.push(PageEntry {
            offset,
            len,
            hash: fnv1a32(blob),
        });
        offset = offset
            .checked_add(u64::from(len))
            .ok_or(RvgError::InvalidPlan("file too large"))?;
    }

    let mut out = Vec::with_capacity(usize::try_from(offset).unwrap_or(0));
    out.extend_from_slice(&MAGIC);
    put_u16(&mut out, VERSION);
    put_u16(
        &mut out,
        u16::try_from(plan.resident_pages.len())
            .map_err(|_| RvgError::InvalidPlan("too many resident pages"))?,
    );
    put_u32(&mut out, to_u32(cluster_count)?);
    put_u32(&mut out, to_u32(plan.pages.len())?);
    put_u32(&mut out, asset.dag.root_count);
    for entry in &entries {
        put_u64(&mut out, entry.offset);
        put_u32(&mut out, entry.len);
        put_u32(&mut out, entry.hash);
    }
    for &page in &plan.cluster_page {
        put_u32(&mut out, page);
    }
    for cluster in &asset.dag.clusters {
        encode_cluster(&mut out, cluster);
    }
    for blob in &page_blobs {
        out.extend_from_slice(blob);
    }
    Ok(out)
}

/// Returns the header length implied by the first 16 bytes of a file.
///
/// A reader can fetch these 16 bytes first, then the rest of the header.
pub fn header_len(prefix: &[u8]) -> Result<usize, RvgError> {
    let mut reader = Reader::new(prefix);
    let magic: [u8; 4] = reader.array()?;
    if magic != MAGIC {
        return Err(RvgError::BadMagic);
    }
    let version = reader.u16()?;
    if version != VERSION {
        return Err(RvgError::UnsupportedVersion(version));
    }
    let _resident = reader.u16()?;
    let cluster_count = reader.u32()?;
    let page_count = reader.u32()?;
    let clusters = usize::try_from(cluster_count).map_err(|_| RvgError::Truncated)?;
    let pages = usize::try_from(page_count).map_err(|_| RvgError::Truncated)?;
    header_len_from_counts(clusters, pages).ok_or(RvgError::Corrupt("header size overflows"))
}

/// Parses the header and page table from `head`, which must contain at least
/// `header_len(..)` bytes from the start of the file.
pub fn read_index(head: &[u8]) -> Result<RvgIndex, RvgError> {
    let total = header_len(head)?;
    let bytes = head.get(..total).ok_or(RvgError::Truncated)?;
    let mut reader = Reader::new(bytes);
    let _magic: [u8; 4] = reader.array()?;
    let _version = reader.u16()?;
    let resident = u32::from(reader.u16()?);
    let cluster_count = to_usize(reader.u32()?)?;
    let page_count = to_usize(reader.u32()?)?;
    let root_count = reader.u32()?;

    if usize::try_from(resident).map_err(|_| RvgError::Truncated)? > page_count {
        return Err(RvgError::Corrupt("more resident pages than pages"));
    }
    let mut entries = Vec::with_capacity(page_count);
    for _ in 0..page_count {
        entries.push(PageEntry {
            offset: reader.u64()?,
            len: reader.u32()?,
            hash: reader.u32()?,
        });
    }
    let mut cluster_page = Vec::with_capacity(cluster_count);
    for _ in 0..cluster_count {
        let page = reader.u32()?;
        if to_usize(page)? >= page_count {
            return Err(RvgError::Corrupt("cluster refers to a missing page"));
        }
        cluster_page.push(page);
    }
    let mut clusters = Vec::with_capacity(cluster_count);
    for _ in 0..cluster_count {
        clusters.push(decode_cluster(&mut reader)?);
    }
    if root_count as usize > clusters.len() {
        return Err(RvgError::Corrupt("root count exceeds cluster count"));
    }
    Ok(RvgIndex {
        dag: ClusterDag {
            clusters,
            root_count,
        },
        cluster_page,
        entries,
        resident_page_count: resident,
        header_len: total,
    })
}

/// Decodes page `page` from `bytes`, which must be exactly the page's bytes
/// as given by its page table entry. The hash is verified first.
pub fn decode_page(bytes: &[u8], index: &RvgIndex, page: u32) -> Result<PageData, RvgError> {
    let entry = index
        .page_entry(page)
        .ok_or(RvgError::PageOutOfRange(page))?;
    if bytes.len() != entry.len as usize {
        return Err(RvgError::Corrupt(
            "page length does not match the page table",
        ));
    }
    if fnv1a32(bytes) != entry.hash {
        return Err(RvgError::HashMismatch { page });
    }
    let mut reader = Reader::new(bytes);
    let vertex_count = to_usize(reader.u32()?)?;
    let index_count = to_usize(reader.u32()?)?;
    let cluster_count = to_usize(reader.u32()?)?;
    let needed = PAGE_HEADER_BYTES
        .checked_add(
            cluster_count
                .checked_mul(PAGE_CLUSTER_BYTES)
                .ok_or(RvgError::Truncated)?,
        )
        .and_then(|n| n.checked_add(vertex_count.checked_mul(VERTEX_BYTES)?))
        .and_then(|n| n.checked_add(index_count.checked_mul(INDEX_BYTES)?))
        .ok_or(RvgError::Truncated)?;
    if needed != bytes.len() {
        return Err(RvgError::Corrupt("page sizes do not add up"));
    }

    let mut clusters = Vec::with_capacity(cluster_count);
    for _ in 0..cluster_count {
        let cluster = decode_page_cluster(&mut reader, index, index_count)?;
        clusters.push(cluster);
    }
    let mut vertices = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        vertices.push(read_vertex(&mut reader)?);
    }
    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        let local = reader.u32()?;
        if to_usize(local)? >= vertex_count {
            return Err(RvgError::Corrupt("index outside the page vertices"));
        }
        indices.push(local);
    }
    Ok(PageData {
        clusters,
        vertices,
        indices,
    })
}

/// Checks the plan against the asset before anything is written.
fn validate_plan(asset: &VGDagAsset, plan: &PagePlan) -> Result<(), RvgError> {
    if plan.cluster_page.len() != asset.dag.clusters.len() {
        return Err(RvgError::InvalidPlan(
            "cluster_page length differs from cluster count",
        ));
    }
    if plan
        .cluster_page
        .iter()
        .any(|&p| to_usize(p).map_or(true, |p| p >= plan.pages.len()))
    {
        return Err(RvgError::InvalidPlan(
            "cluster_page refers to a missing page",
        ));
    }
    let resident_ok = plan
        .resident_pages
        .iter()
        .enumerate()
        .all(|(i, &page)| to_usize(page).map_or(false, |p| p == i));
    if !resident_ok || plan.resident_pages.len() > plan.pages.len() {
        return Err(RvgError::InvalidPlan(
            "resident pages must be the leading pages",
        ));
    }
    Ok(())
}

/// Builds the bytes of one self-contained page.
///
/// Page layout: `vertex_count u32, index_count u32, cluster_count u32`, then
/// `cluster_count` records `{cluster u32, index_offset u32, index_count u32}`,
/// then the vertices (11 f32 each) and the local indices (u32 each).
fn encode_page(asset: &VGDagAsset, plan: &PagePlan, span: PageSpan) -> Result<Vec<u8>, RvgError> {
    let first = span.first_cluster as usize;
    let end = first
        .checked_add(span.cluster_count as usize)
        .ok_or(RvgError::InvalidPlan("page span overflows"))?;
    let cluster_ids = plan
        .order
        .get(first..end)
        .ok_or(RvgError::InvalidPlan("page span outside the cluster order"))?;

    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut records: Vec<(u32, u32, u32)> = Vec::with_capacity(cluster_ids.len());
    for &cluster_id in cluster_ids {
        let cluster = asset
            .dag
            .clusters
            .get(cluster_id as usize)
            .ok_or(RvgError::InvalidPlan("page refers to a missing cluster"))?;
        let start = cluster.index_offset as usize;
        let stop = start
            .checked_add(cluster.index_count as usize)
            .ok_or(RvgError::InvalidPlan("cluster index range overflows"))?;
        let source = asset.indices.get(start..stop).ok_or(RvgError::InvalidPlan(
            "cluster index range outside the asset",
        ))?;
        records.push((cluster_id, to_u32(indices.len())?, cluster.index_count));
        for &global in source {
            let local = match remap.get(&global) {
                Some(&local) => local,
                None => {
                    let vertex = asset
                        .vertices
                        .get(global as usize)
                        .ok_or(RvgError::InvalidPlan("index outside the asset vertices"))?;
                    let local = to_u32(vertices.len())?;
                    vertices.push(*vertex);
                    remap.insert(global, local);
                    local
                }
            };
            indices.push(local);
        }
    }

    let mut out = Vec::with_capacity(
        PAGE_HEADER_BYTES
            + records.len() * PAGE_CLUSTER_BYTES
            + vertices.len() * VERTEX_BYTES
            + indices.len() * INDEX_BYTES,
    );
    put_u32(&mut out, to_u32(vertices.len())?);
    put_u32(&mut out, to_u32(indices.len())?);
    put_u32(&mut out, to_u32(records.len())?);
    for (cluster, offset, count) in records {
        put_u32(&mut out, cluster);
        put_u32(&mut out, offset);
        put_u32(&mut out, count);
    }
    for vertex in &vertices {
        put_vertex(&mut out, vertex);
    }
    for &index in &indices {
        put_u32(&mut out, index);
    }
    Ok(out)
}

fn encode_cluster(out: &mut Vec<u8>, cluster: &DagCluster) {
    put_u32(out, cluster.index_offset);
    put_u32(out, cluster.index_count);
    put_f32s(out, &cluster.center);
    put_f32(out, cluster.radius);
    put_f32s(out, &cluster.lod_center);
    put_f32(out, cluster.lod_radius);
    put_f32(out, cluster.error);
    put_f32s(out, &cluster.parent_lod_center);
    put_f32(out, cluster.parent_lod_radius);
    put_f32(out, cluster.parent_error);
    put_u32(out, cluster.level);
    put_u32(out, cluster.group);
}

fn decode_cluster(reader: &mut Reader<'_>) -> Result<DagCluster, RvgError> {
    Ok(DagCluster {
        index_offset: reader.u32()?,
        index_count: reader.u32()?,
        center: reader.f32x3()?,
        radius: reader.f32()?,
        lod_center: reader.f32x3()?,
        lod_radius: reader.f32()?,
        error: reader.f32()?,
        parent_lod_center: reader.f32x3()?,
        parent_lod_radius: reader.f32()?,
        parent_error: reader.f32()?,
        level: reader.u32()?,
        group: reader.u32()?,
    })
}

fn decode_page_cluster(
    reader: &mut Reader<'_>,
    index: &RvgIndex,
    index_count: usize,
) -> Result<PageCluster, RvgError> {
    let cluster = reader.u32()?;
    if to_usize(cluster)? >= index.dag.clusters.len() {
        return Err(RvgError::Corrupt("page refers to an unknown cluster"));
    }
    let index_offset = reader.u32()?;
    let index_count_of_cluster = reader.u32()?;
    let end = to_usize(index_offset)?
        .checked_add(to_usize(index_count_of_cluster)?)
        .ok_or(RvgError::Corrupt("cluster index range overflows"))?;
    if end > index_count {
        return Err(RvgError::Corrupt("cluster index range outside the page"));
    }
    Ok(PageCluster {
        cluster,
        index_offset,
        index_count: index_count_of_cluster,
    })
}

fn read_vertex(reader: &mut Reader<'_>) -> Result<Vertex, RvgError> {
    Ok(Vertex {
        position: reader.f32x3()?,
        normal: reader.f32x3()?,
        uv: [reader.f32()?, reader.f32()?],
        tangent: reader.f32x3()?,
    })
}

fn put_vertex(out: &mut Vec<u8>, vertex: &Vertex) {
    put_f32s(out, &vertex.position);
    put_f32s(out, &vertex.normal);
    put_f32s(out, &vertex.uv);
    put_f32s(out, &vertex.tangent);
}

/// Header length for the given counts, or `None` on overflow.
fn header_len_from_counts(clusters: usize, pages: usize) -> Option<usize> {
    let table = pages.checked_mul(PAGE_ENTRY_BYTES)?;
    let per_cluster = CLUSTER_PAGE_BYTES + CLUSTER_RECORD_BYTES;
    let cluster_bytes = clusters.checked_mul(per_cluster)?;
    HEADER_PREFIX_BYTES
        .checked_add(table)?
        .checked_add(cluster_bytes)
}

/// FNV-1a 32-bit hash used to detect corrupted pages.
fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn to_u32(value: usize) -> Result<u32, RvgError> {
    u32::try_from(value).map_err(|_| RvgError::InvalidPlan("value exceeds u32"))
}

fn to_usize(value: u32) -> Result<usize, RvgError> {
    usize::try_from(value).map_err(|_| RvgError::Truncated)
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_f32s(out: &mut Vec<u8>, values: &[f32]) {
    for &value in values {
        put_f32(out, value);
    }
}

/// Bounds-checked little-endian reader over a byte slice.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], RvgError> {
        let end = self.pos.checked_add(len).ok_or(RvgError::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(RvgError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], RvgError> {
        let slice = self.take(N)?;
        slice.try_into().map_err(|_| RvgError::Truncated)
    }

    fn u16(&mut self) -> Result<u16, RvgError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, RvgError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, RvgError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, RvgError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn f32x3(&mut self) -> Result<[f32; 3], RvgError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
}

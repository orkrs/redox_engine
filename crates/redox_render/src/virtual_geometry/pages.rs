//! Page layout of a cluster DAG (VG-03 step 1).
//!
//! The DAG is split into pages of roughly `PAGE_BYTES` each. A group of
//! clusters (same `group` id) never straddles two pages. The coarsest levels
//! are resident: they are packed first, into the leading pages.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use super::dag::{ClusterDag, DagCluster};

/// Default page size in bytes (64 KiB).
pub const PAGE_BYTES: usize = 64 * 1024;

/// `group` value of ungrouped clusters (leaves).
const UNGROUPED: u32 = u32::MAX;

/// Number of top hierarchy levels that are always resident.
const RESIDENT_LEVELS: u64 = 2;

/// Which clusters live in which page.
///
/// `order` lists all cluster ids in page order; page `p` owns the slice
/// `order[pages[p].first_cluster .. first_cluster + cluster_count]`.
/// Resident pages are always `0..resident_pages.len()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PagePlan {
    /// Page spans, in page-id order.
    pub pages: Vec<PageSpan>,
    /// Page id for every cluster (indexed by cluster id).
    pub cluster_page: Vec<u32>,
    /// Ids of pages that must always be loaded (the coarse levels).
    pub resident_pages: Vec<u32>,
    /// Cluster ids sorted by page; see the type docs.
    pub order: Vec<u32>,
}

/// Location and size of one page inside `PagePlan::order`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSpan {
    /// Index of the first cluster of this page in `PagePlan::order`.
    pub first_cluster: u32,
    /// Number of clusters in this page.
    pub cluster_count: u32,
    /// Sum of the cluster sizes in bytes.
    pub bytes: u32,
}

/// Errors of the page planner.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PageError {
    /// `bytes_per_cluster` does not have one entry per cluster.
    #[error("bytes_per_cluster has {got} entries for {expected} clusters")]
    SizeMismatch {
        /// Number of clusters in the DAG.
        expected: usize,
        /// Number of size entries supplied.
        got: usize,
    },
    /// A group (or a single cluster) does not fit into one page.
    #[error("group {group} needs {bytes} bytes but a page holds {page_bytes}")]
    GroupTooLarge {
        /// Group id (`u32::MAX` for a single ungrouped cluster).
        group: u32,
        /// Size of the group in bytes.
        bytes: usize,
        /// Page size that was requested.
        page_bytes: usize,
    },
    /// The DAG or the plan has more items than the `u32` ids can address.
    #[error("too many clusters or pages for u32 ids")]
    TooManyItems,
}

/// Packs the clusters of `dag` into pages of at most `page_bytes` each.
///
/// `bytes_per_cluster[i]` is the payload size of cluster `i`. Clusters with
/// `group == u32::MAX` are independent units. The result is deterministic:
/// units are ordered by (level descending, group id, first cluster index).
/// A level is resident when it is one of the two top levels of the DAG.
pub fn plan_pages(
    dag: &ClusterDag,
    bytes_per_cluster: &[u32],
    page_bytes: usize,
) -> Result<PagePlan, PageError> {
    let count = dag.clusters.len();
    if bytes_per_cluster.len() != count {
        return Err(PageError::SizeMismatch {
            expected: count,
            got: bytes_per_cluster.len(),
        });
    }
    if u32::try_from(count).is_err() {
        return Err(PageError::TooManyItems);
    }
    let max_level = dag.clusters.iter().map(|c| c.level).max().unwrap_or(0);
    let mut units = build_units(&dag.clusters, bytes_per_cluster);
    units.sort_by_key(|u| (Reverse(u.level), u.group, u.first));
    pack(&units, max_level, page_bytes, count)
}

/// A set of clusters that must be placed in the same page.
struct Unit {
    level: u32,
    group: u32,
    first: u32,
    members: Vec<u32>,
    bytes: usize,
}

/// Groups clusters into units: one unit per group id, one per ungrouped cluster.
fn build_units(clusters: &[DagCluster], sizes: &[u32]) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    let mut unit_of_group: BTreeMap<u32, usize> = BTreeMap::new();
    for (index, (cluster, &size)) in clusters.iter().zip(sizes).enumerate() {
        let id = index as u32;
        let size = size as usize;
        if cluster.group != UNGROUPED {
            if let Some(&slot) = unit_of_group.get(&cluster.group) {
                if let Some(unit) = units.get_mut(slot) {
                    unit.members.push(id);
                    unit.bytes = unit.bytes.saturating_add(size);
                }
                continue;
            }
            unit_of_group.insert(cluster.group, units.len());
        }
        units.push(Unit {
            level: cluster.level,
            group: cluster.group,
            first: id,
            members: vec![id],
            bytes: size,
        });
    }
    units
}

/// True when a cluster of `level` belongs to the two top levels of the DAG.
fn is_resident(level: u32, max_level: u32) -> bool {
    u64::from(level) + RESIDENT_LEVELS > u64::from(max_level)
}

/// The page currently being filled.
#[derive(Default)]
struct OpenPage {
    members: Vec<u32>,
    bytes: usize,
    resident: bool,
}

/// Fills pages with the sorted units; a page is closed when full or when the
/// residency class changes, so resident pages come first.
fn pack(
    units: &[Unit],
    max_level: u32,
    page_bytes: usize,
    cluster_count: usize,
) -> Result<PagePlan, PageError> {
    let mut plan = PagePlan {
        pages: Vec::new(),
        cluster_page: vec![0; cluster_count],
        resident_pages: Vec::new(),
        order: Vec::with_capacity(cluster_count),
    };
    let mut open = OpenPage::default();
    for unit in units {
        if unit.bytes > page_bytes {
            return Err(PageError::GroupTooLarge {
                group: unit.group,
                bytes: unit.bytes,
                page_bytes,
            });
        }
        let resident = is_resident(unit.level, max_level);
        let is_full = open.bytes.saturating_add(unit.bytes) > page_bytes;
        if !open.members.is_empty() && (is_full || open.resident != resident) {
            flush(&mut plan, &mut open)?;
        }
        open.resident = resident;
        open.bytes = open.bytes.saturating_add(unit.bytes);
        open.members.extend_from_slice(&unit.members);
    }
    flush(&mut plan, &mut open)?;
    Ok(plan)
}

/// Writes the open page into `plan` and clears it.
fn flush(plan: &mut PagePlan, open: &mut OpenPage) -> Result<(), PageError> {
    if open.members.is_empty() {
        return Ok(());
    }
    let page = u32::try_from(plan.pages.len()).map_err(|_| PageError::TooManyItems)?;
    let first_cluster = u32::try_from(plan.order.len()).map_err(|_| PageError::TooManyItems)?;
    let cluster_count = u32::try_from(open.members.len()).map_err(|_| PageError::TooManyItems)?;
    let bytes = u32::try_from(open.bytes).map_err(|_| PageError::TooManyItems)?;
    for &cluster in &open.members {
        if let Some(slot) = plan.cluster_page.get_mut(cluster as usize) {
            *slot = page;
        }
    }
    plan.order.extend_from_slice(&open.members);
    plan.pages.push(PageSpan {
        first_cluster,
        cluster_count,
        bytes,
    });
    if open.resident {
        plan.resident_pages.push(page);
    }
    open.members.clear();
    open.bytes = 0;
    Ok(())
}

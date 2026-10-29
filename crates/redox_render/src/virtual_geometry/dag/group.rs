//! Grouping of neighbouring clusters (pure, no meshopt).

use std::collections::{BTreeMap, HashMap};

/// Largest group a cluster may be merged into.
const MAX_GROUP_SIZE: usize = 6;
/// Smallest allowed target group size.
const MIN_GROUP_SIZE: usize = 2;

/// Counts, for each cluster pair, the boundary edges they share. Input: per cluster a list of undirected edges (pairs of position-welded vertex ids).
///
/// Output: for each cluster, its neighbours as `(neighbour, shared_edge_count)`, sorted by neighbour index.
/// Duplicate edges inside one cluster are counted once.
pub fn adjacency(cluster_edges: &[Vec<(u32, u32)>]) -> Vec<Vec<(u32, u32)>> {
    let mut owners: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (ci, edges) in cluster_edges.iter().enumerate() {
        for &(a, b) in edges {
            let key = (a.min(b), a.max(b));
            let list = owners.entry(key).or_default();
            if list.last() != Some(&(ci as u32)) {
                list.push(ci as u32);
            }
        }
    }

    let mut pairs: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for list in owners.values() {
        for (k, &a) in list.iter().enumerate() {
            for &b in &list[k + 1..] {
                *pairs.entry((a, b)).or_insert(0) += 1;
            }
        }
    }

    let mut adj: Vec<Vec<(u32, u32)>> = vec![Vec::new(); cluster_edges.len()];
    for (&(a, b), &count) in &pairs {
        adj[a as usize].push((b, count));
        adj[b as usize].push((a, count));
    }
    for list in &mut adj {
        list.sort_unstable_by_key(|&(n, _)| n);
    }
    adj
}

/// Partitions clusters into groups of `target` (2..=6) by shared edges. `shared_edges[i]` lists `(j, count)` neighbours of cluster i. Deterministic.
///
/// Greedy: the lowest unassigned cluster seeds a group; the group grows by the unassigned
/// neighbour with the most shared edges to the group (ties: more adjacent members, then the
/// smaller index). A single-cluster group left over is merged into the neighbouring group
/// with the most shared edges when that group has at most 5 members.
pub fn group_clusters(
    cluster_count: usize,
    shared_edges: &[Vec<(u32, u32)>],
    target: usize,
) -> Vec<Vec<u32>> {
    let target = target.clamp(MIN_GROUP_SIZE, MAX_GROUP_SIZE);
    let mut group_of = vec![usize::MAX; cluster_count];
    let mut groups: Vec<Vec<u32>> = Vec::new();

    for seed in 0..cluster_count {
        if group_of[seed] != usize::MAX {
            continue;
        }
        let gid = groups.len();
        let mut members: Vec<u32> = vec![seed as u32];
        group_of[seed] = gid;
        while members.len() < target {
            let Some(next) = best_candidate(&members, shared_edges, &group_of) else {
                break;
            };
            group_of[next as usize] = gid;
            members.push(next);
        }
        members.sort_unstable();
        groups.push(members);
    }

    merge_singletons(&mut groups, &mut group_of, shared_edges);
    groups.retain(|g| !g.is_empty());
    groups
}

/// Unassigned neighbour of the group with the highest `(shared edges, adjacent members)`; ties go to the smaller index.
fn best_candidate(
    members: &[u32],
    shared_edges: &[Vec<(u32, u32)>],
    group_of: &[usize],
) -> Option<u32> {
    // candidate -> (shared edges to group, adjacent members)
    let mut scores: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for &m in members {
        for &(nb, count) in &shared_edges[m as usize] {
            if group_of[nb as usize] != usize::MAX {
                continue;
            }
            let entry = scores.entry(nb).or_insert((0, 0));
            entry.0 += count;
            entry.1 += 1;
        }
    }
    let mut best: Option<(u32, (u32, u32))> = None;
    for (&cand, &score) in &scores {
        match best {
            Some((_, b)) if score <= b => {}
            _ => best = Some((cand, score)),
        }
    }
    best.map(|(c, _)| c)
}

/// Merges every single-cluster group into the neighbouring group with the most shared edges, if that group has at most 5 members.
fn merge_singletons(
    groups: &mut [Vec<u32>],
    group_of: &mut [usize],
    shared_edges: &[Vec<(u32, u32)>],
) {
    for gid in 0..groups.len() {
        if groups[gid].len() != 1 {
            continue;
        }
        let cluster = groups[gid][0];
        let mut best: Option<(usize, u32)> = None; // (group, shared edges)
        for &(nb, count) in &shared_edges[cluster as usize] {
            let other = group_of[nb as usize];
            if other == gid || other == usize::MAX || groups[other].len() > MAX_GROUP_SIZE - 1 {
                continue;
            }
            match best {
                Some((_, b)) if count <= b => {}
                _ => best = Some((other, count)),
            }
        }
        if let Some((other, _)) = best {
            groups[other].push(cluster);
            groups[other].sort_unstable();
            group_of[cluster as usize] = other;
            groups[gid].clear();
        }
    }
}

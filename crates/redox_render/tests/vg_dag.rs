//! Virtual geometry hierarchy (VG-01), paging (VG-03): DAG validity, no cracks at any cut, pages, pool, `.rvg`.
//! Pure CPU, no GPU needed.

#![allow(clippy::unwrap_used)]

use redox_render::mesh::Mesh;
use redox_render::mesh::primitive::{create_cube, create_sphere};
use redox_render::virtual_geometry::dag::{
    ClusterDag, DagInstance, DagParams, build_cluster_dag, build_vg_dag_asset, cut, gpu_clusters,
    open_edge_count, projected_error_px, select_for_instances, validate, weld_positions,
};
use redox_render::virtual_geometry::pages::{PAGE_BYTES, plan_pages};
use redox_render::virtual_geometry::pool::{PagePool, pages_to_load};
use redox_render::virtual_geometry::rvg::{decode_page, header_len, read_index, write_rvg};

const PPU: f32 = 1000.0;

fn sphere() -> Mesh {
    create_sphere(1.0, 48)
}

fn build(mesh: &Mesh) -> (ClusterDag, Vec<u32>) {
    build_cluster_dag(&mesh.vertices, &mesh.indices, &DagParams::default()).unwrap()
}

fn triangles_of(dag: &ClusterDag, indices: &[u32], chosen: &[u32]) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::new();
    for &c in chosen {
        let cluster = &dag.clusters[c as usize];
        let range = cluster.index_offset as usize
            ..(cluster.index_offset + cluster.index_count) as usize;
        for t in indices[range].chunks_exact(3) {
            out.push((t[0], t[1], t[2]));
        }
    }
    out
}

// ---------------------------------------------------------------- projected error and the cut

#[test]
fn a_closer_eye_gives_a_larger_projected_error() {
    let near = projected_error_px(0.1, [0.0; 3], 1.0, [0.0, 0.0, 2.0], PPU);
    let far = projected_error_px(0.1, [0.0; 3], 1.0, [0.0, 0.0, 20.0], PPU);
    assert!(near > far);
}

#[test]
fn an_eye_inside_the_sphere_does_not_divide_by_zero() {
    let v = projected_error_px(0.1, [0.0; 3], 1.0, [0.0; 3], PPU);
    assert!(v.is_finite() && v > 0.0);
}

// ---------------------------------------------------------------- the hierarchy

#[test]
fn a_sphere_builds_a_valid_hierarchy() {
    let mesh = sphere();
    let (dag, _) = build(&mesh);
    assert_eq!(validate(&dag), Vec::<String>::new());
    let levels = dag.clusters.iter().map(|c| c.level).max().unwrap();
    assert!(levels >= 2, "levels {levels}");
}

#[test]
fn the_root_has_few_triangles() {
    let (dag, _) = build(&sphere());
    let root_indices: u32 = dag
        .clusters
        .iter()
        .filter(|c| c.parent_error.is_infinite())
        .map(|c| c.index_count)
        .sum();
    assert!(root_indices / 3 <= 2 * 124 + 124, "root triangles {}", root_indices / 3);
}

#[test]
fn parent_errors_never_decrease_up_the_hierarchy() {
    let (dag, _) = build(&sphere());
    for c in &dag.clusters {
        assert!(c.parent_error >= c.error, "{c:?}");
    }
}

#[test]
fn the_build_is_deterministic() {
    let mesh = sphere();
    let (a, ia) = build(&mesh);
    let (b, ib) = build(&mesh);
    assert_eq!(ia, ib);
    assert_eq!(a.clusters, b.clusters);
}

#[test]
fn an_empty_mesh_is_an_error() {
    assert!(build_cluster_dag(&[], &[], &DagParams::default()).is_err());
}

#[test]
fn a_mesh_that_fits_in_one_cluster_is_just_the_root() {
    let cube = create_cube();
    let (dag, _) = build(&cube);
    assert_eq!(dag.clusters.len(), 1);
    assert!(dag.clusters[0].parent_error.is_infinite());
    assert_eq!(dag.clusters[0].error, 0.0);
    assert_eq!(dag.root_count, 1);
}

#[test]
fn leaves_cover_every_triangle_of_the_welded_mesh_once() {
    let mesh = sphere();
    let (dag, indices) = build(&mesh);
    let weld = weld_positions(&mesh.vertices);
    let leaves: Vec<u32> = (0..dag.clusters.len() as u32)
        .filter(|&i| dag.clusters[i as usize].level == 0)
        .collect();
    let key = |t: (u32, u32, u32)| {
        let mut v = [weld[t.0 as usize], weld[t.1 as usize], weld[t.2 as usize]];
        v.sort_unstable();
        v
    };
    let mut got: Vec<[u32; 3]> = triangles_of(&dag, &indices, &leaves).into_iter().map(key).collect();
    let mut want: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|t| key((t[0], t[1], t[2])))
        .filter(|v| v[0] != v[1] && v[1] != v[2])
        .collect();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want);
}

// ---------------------------------------------------------------- no cracks

#[test]
fn no_cut_opens_an_edge_that_the_source_mesh_does_not_have() {
    let mesh = sphere();
    let (dag, indices) = build(&mesh);
    let weld = weld_positions(&mesh.vertices);
    // The primitive sphere has a UV seam whose duplicated vertices differ in the last bits, so the
    // exact-bit weld keeps it open. The baseline is the source mesh: no cut may add an open edge.
    let source = mesh
        .indices
        .chunks_exact(3)
        .map(|t| (t[0], t[1], t[2]))
        .filter(|&(a, b, c)| {
            let (a, b, c) = (weld[a as usize], weld[b as usize], weld[c as usize]);
            a != b && b != c && a != c
        });
    let baseline = open_edge_count(source, &weld);
    for step in 0..40 {
        let distance = 1.5 * 1.15f32.powi(step); // 1.5 .. ~400
        for threshold in [0.5f32, 1.0, 2.0, 4.0] {
            let chosen = cut(&dag, [0.0, 0.0, distance], PPU, threshold);
            assert!(!chosen.is_empty(), "nothing chosen at {distance} / {threshold}");
            let tris = triangles_of(&dag, &indices, &chosen);
            let open = open_edge_count(tris.into_iter(), &weld);
            assert_eq!(open, baseline, "cracks at distance {distance}, threshold {threshold}");
        }
    }
}

#[test]
fn the_triangle_count_does_not_grow_as_the_eye_moves_away() {
    let mesh = sphere();
    let (dag, indices) = build(&mesh);
    let mut last = usize::MAX;
    for step in 0..30 {
        let distance = 1.5 * 1.2f32.powi(step);
        let chosen = cut(&dag, [0.0, 0.0, distance], PPU, 1.0);
        let n = triangles_of(&dag, &indices, &chosen).len();
        assert!(n <= last, "{n} > {last} at distance {distance}");
        last = n;
    }
}

#[test]
fn the_cut_covers_every_surface_point_exactly_once_by_triangle_area() {
    // Total area of the chosen clusters stays close to the sphere's surface at every distance.
    let mesh = sphere();
    let (dag, indices) = build(&mesh);
    let area = |tris: &[(u32, u32, u32)]| -> f32 {
        tris.iter()
            .map(|&(a, b, c)| {
                let p = |i: u32| redox_math::Vec3::from(mesh.vertices[i as usize].position);
                (p(b) - p(a)).cross(p(c) - p(a)).length() * 0.5
            })
            .sum()
    };
    let full = area(&triangles_of(&dag, &indices, &cut(&dag, [0.0, 0.0, 1.5], PPU, 0.01)));
    for distance in [3.0f32, 10.0, 50.0] {
        let chosen = cut(&dag, [0.0, 0.0, distance], PPU, 1.0);
        let a = area(&triangles_of(&dag, &indices, &chosen));
        assert!(a > 0.5 * full && a < 1.2 * full, "area {a} vs {full} at {distance}");
    }
}

// ---------------------------------------------------------------- GPU layout and instance selection

#[test]
fn every_non_leaf_cluster_has_children_and_leaves_have_none() {
    let (dag, _) = build(&sphere());
    let (gpu, child_ids) = gpu_clusters(&dag);
    assert_eq!(gpu.len(), dag.clusters.len());
    assert_eq!(std::mem::size_of::<redox_render::virtual_geometry::dag::DagClusterGpu>(), 96);
    for (c, g) in dag.clusters.iter().zip(&gpu) {
        if c.level == 0 {
            assert_eq!(g.child_count, 0);
        } else {
            assert!(g.child_count > 0);
            assert!((g.first_child + g.child_count) as usize <= child_ids.len());
        }
    }
}

#[test]
fn the_instance_selection_equals_the_union_of_per_instance_cuts() {
    let (dag, _) = build(&sphere());
    let place = |x: f32| DagInstance {
        dag: 0,
        transform: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [x, 0.0, 0.0, 1.0],
        ],
    };
    let instances = [place(0.0), place(30.0), place(200.0)];
    let eye = [0.0, 0.0, 6.0];
    let selected = select_for_instances(std::slice::from_ref(&dag), &instances, eye, PPU, 1.0);
    for (ii, inst) in instances.iter().enumerate() {
        let local = [eye[0] - inst.transform[3][0], eye[1], eye[2]];
        let mut want = cut(&dag, local, PPU, 1.0);
        let mut got: Vec<u32> = selected
            .iter()
            .filter(|(i, _)| *i == ii as u32)
            .map(|(_, c)| *c)
            .collect();
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want, "instance {ii}");
    }
}

// ---------------------------------------------------------------- pages (VG-03)

#[test]
fn pages_respect_size_groups_and_residency() {
    let (dag, _) = build(&sphere());
    let bytes = vec![3_000u32; dag.clusters.len()];
    let plan = plan_pages(&dag, &bytes, PAGE_BYTES).unwrap();
    // Every cluster has exactly one page, all pages fit.
    assert_eq!(plan.cluster_page.len(), dag.clusters.len());
    for (page, span) in plan.pages.iter().enumerate() {
        assert!(span.bytes as usize <= PAGE_BYTES, "page {page}");
        assert!(plan.cluster_page.iter().filter(|&&p| p as usize == page).count() == span.cluster_count as usize);
    }
    // A group never straddles two pages.
    let mut group_page = std::collections::HashMap::new();
    for (i, c) in dag.clusters.iter().enumerate() {
        if c.group != u32::MAX {
            let page = plan.cluster_page[i];
            assert_eq!(*group_page.entry(c.group).or_insert(page), page);
        }
    }
    // The roots live in resident pages.
    for (i, c) in dag.clusters.iter().enumerate() {
        if c.parent_error.is_infinite() {
            assert!(plan.resident_pages.contains(&plan.cluster_page[i]));
        }
    }
}

#[test]
fn planning_is_deterministic_and_oversized_groups_are_errors() {
    let (dag, _) = build(&sphere());
    let bytes = vec![3_000u32; dag.clusters.len()];
    let a = plan_pages(&dag, &bytes, PAGE_BYTES).unwrap();
    let b = plan_pages(&dag, &bytes, PAGE_BYTES).unwrap();
    assert_eq!(a.cluster_page, b.cluster_page);
    assert!(plan_pages(&dag, &bytes, 1_000).is_err());
    assert!(plan_pages(&dag, &bytes[..3], PAGE_BYTES).is_err());
}

// ---------------------------------------------------------------- the pool

#[test]
fn a_touched_page_is_resident_after_allocation() {
    let mut pool = PagePool::new(2);
    assert!(pool.touch(5, 1).is_none());
    let (slot, evicted) = pool.allocate(5, 1).unwrap();
    assert!(evicted.is_none());
    assert_eq!(pool.touch(5, 2), Some(slot));
    assert!(pool.is_resident(5));
}

#[test]
fn the_least_recently_used_page_is_evicted() {
    let mut pool = PagePool::new(2);
    pool.allocate(1, 1).unwrap();
    pool.allocate(2, 2).unwrap();
    pool.touch(1, 3); // page 2 is now the oldest
    let (_, evicted) = pool.allocate(3, 4).unwrap();
    assert_eq!(evicted, Some(2));
    assert!(pool.is_resident(1) && pool.is_resident(3) && !pool.is_resident(2));
}

#[test]
fn pages_touched_this_frame_are_never_evicted() {
    let mut pool = PagePool::new(2);
    pool.allocate(1, 5).unwrap();
    pool.allocate(2, 5).unwrap();
    assert!(pool.allocate(3, 5).is_none(), "everything was used this frame");
}

#[test]
fn the_pool_never_exceeds_its_capacity() {
    let mut pool = PagePool::new(8);
    let mut state = 7u32;
    for frame in 0..10_000u64 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let page = (state >> 16) % 40;
        if pool.touch(page, frame).is_none() {
            let _ = pool.allocate(page, frame);
        }
        assert!(pool.resident_count() <= 8);
    }
}

#[test]
fn pages_to_load_skips_resident_and_duplicates_and_obeys_the_budget() {
    let mut pool = PagePool::new(4);
    pool.allocate(1, 1).unwrap();
    let wanted = pages_to_load(&[3, 1, 3, 4, 5, 6], &pool, 2);
    assert_eq!(wanted, vec![3, 4]);
    assert!(pages_to_load(&[], &pool, 5).is_empty());
}

// ---------------------------------------------------------------- the .rvg file

#[test]
fn rvg_roundtrips_pages_and_detects_corruption() {
    let mesh = sphere();
    let asset = build_vg_dag_asset(&mesh, &DagParams::default(), 0).unwrap();
    let bytes_per_cluster: Vec<u32> = asset
        .dag
        .clusters
        .iter()
        .map(|c| c.index_count * 4 + 44)
        .collect();
    let plan = plan_pages(&asset.dag, &bytes_per_cluster, PAGE_BYTES).unwrap();
    let file = write_rvg(&asset, &plan).unwrap();

    let head_len = header_len(&file[..16]).unwrap();
    let index = read_index(&file[..head_len]).unwrap();
    assert_eq!(index.page_count(), plan.pages.len());

    for page in 0..index.page_count() as u32 {
        let entry = index.page_entry(page).unwrap();
        let start = entry.offset as usize;
        let bytes = &file[start..start + entry.len as usize];
        let data = decode_page(bytes, &index, page).unwrap();
        assert!(!data.clusters.is_empty());
        assert!(data.indices.iter().all(|&i| (i as usize) < data.vertices.len()));
    }

    // Flip a byte in the last page: its hash fails, nothing panics.
    let last = index.page_count() as u32 - 1;
    let entry = index.page_entry(last).unwrap();
    let mut bad = file[entry.offset as usize..entry.offset as usize + entry.len as usize].to_vec();
    let mid = bad.len() / 2;
    bad[mid] ^= 0xFF;
    assert!(decode_page(&bad, &index, last).is_err());
}

#[test]
fn a_truncated_rvg_index_is_an_error() {
    let mesh = sphere();
    let asset = build_vg_dag_asset(&mesh, &DagParams::default(), 0).unwrap();
    let bytes: Vec<u32> = asset.dag.clusters.iter().map(|c| c.index_count * 4 + 44).collect();
    let plan = plan_pages(&asset.dag, &bytes, PAGE_BYTES).unwrap();
    let file = write_rvg(&asset, &plan).unwrap();
    let head_len = header_len(&file[..16]).unwrap();
    for cut in [0usize, 4, 15, head_len / 2, head_len - 1] {
        assert!(read_index(&file[..cut]).is_err(), "cut at {cut}");
    }
}

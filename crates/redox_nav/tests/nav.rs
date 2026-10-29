//! Navigation (NAV-02..04): `.nav` format, voxel settings, baking, paths, links, queue, blockers and avoidance.
//! Worlds are built by hand from rectangular cells, so the path tests do not depend on the baker.

#![allow(clippy::unwrap_used)]

use redox_math::Vec3;
use redox_nav::{
    AgentClass, AgentParams, AvoidanceParams, BakeError, NavAreaId, NavData, NavError,
    NavFormatError, NavGeometry, NavLink, NavPolygon, NavTile, NavWorld, PathQueue, bake,
    voxel_config,
};

// ---------------------------------------------------------------- helpers

/// A tile of `cols` x `rows` square cells of `cell` metres whose corner is at (`x0`, `z0`).
/// `area(c, r)` returns the area of a cell or `None` to leave it out.
fn grid_tile(
    coord: (i32, i32),
    x0: f32,
    z0: f32,
    cols: u16,
    rows: u16,
    cell: f32,
    area: impl Fn(u16, u16) -> Option<NavAreaId>,
) -> NavTile {
    let stride = cols + 1;
    let mut vertices = Vec::new();
    for r in 0..=rows {
        for c in 0..=cols {
            vertices.push([x0 + f32::from(c) * cell, 0.0, z0 + f32::from(r) * cell]);
        }
    }
    let at = |c: u16, r: u16| r * stride + c;
    let mut polygons = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if let Some(a) = area(c, r) {
                // Counter-clockwise seen from above (+Y): z first, then x.
                polygons.push(NavPolygon {
                    indices: vec![at(c, r), at(c, r + 1), at(c + 1, r + 1), at(c + 1, r)],
                    area: a,
                });
            }
        }
    }
    NavTile {
        coord,
        vertices,
        polygons,
    }
}

fn data(tiles: Vec<NavTile>) -> NavData {
    NavData {
        agent: AgentParams::default(),
        tile_size: 16.0,
        tiles,
    }
}

fn world_of(tiles: Vec<NavTile>) -> NavWorld {
    NavWorld::from_data(&data(tiles)).unwrap()
}

fn walk(_c: u16, _r: u16) -> Option<NavAreaId> {
    Some(NavAreaId::WALKABLE)
}

fn p(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, 0.0, z)
}

fn path_length(path: &[Vec3]) -> f32 {
    path.windows(2).map(|w| (w[1] - w[0]).length()).sum()
}

// ---------------------------------------------------------------- .nav format

fn two_tiles() -> NavData {
    data(vec![
        grid_tile((0, 0), 0.0, 0.0, 2, 2, 1.0, walk),
        grid_tile((1, 0), 2.0, 0.0, 2, 2, 1.0, walk),
    ])
}

#[test]
fn bytes_roundtrip_is_lossless() {
    let d = two_tiles();
    assert_eq!(NavData::from_bytes(&d.to_bytes()).unwrap(), d);
}

#[test]
fn serialisation_is_deterministic_and_sorted() {
    let a = two_tiles();
    let mut b = two_tiles();
    b.tiles.reverse();
    assert_eq!(a.to_bytes(), b.to_bytes());
}

#[test]
fn truncated_data_is_an_error() {
    let bytes = two_tiles().to_bytes();
    for cut in 0..bytes.len() {
        assert!(NavData::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn a_wrong_magic_or_version_is_rejected() {
    let mut bytes = two_tiles().to_bytes();
    bytes[0] ^= 0xFF;
    assert!(matches!(
        NavData::from_bytes(&bytes),
        Err(NavFormatError::BadMagic)
    ));
    let mut bytes = two_tiles().to_bytes();
    bytes[4] = 99;
    assert!(NavData::from_bytes(&bytes).is_err());
}

#[test]
fn an_index_out_of_range_is_rejected() {
    let mut d = two_tiles();
    d.tiles[0].polygons[0].indices[0] = 9999;
    assert!(NavData::from_bytes(&d.to_bytes()).is_err());
}

#[test]
fn a_polygon_with_two_indices_is_rejected() {
    let mut d = two_tiles();
    d.tiles[0].polygons[0].indices.truncate(2);
    assert!(NavData::from_bytes(&d.to_bytes()).is_err());
}

// ---------------------------------------------------------------- voxel settings

#[test]
fn the_default_agent_gives_known_voxel_counts() {
    let v = voxel_config(&AgentParams::default());
    assert!((v.cell_size - 0.2).abs() < 1e-6);
    assert_eq!(v.walkable_radius, 2);
    assert_eq!(v.walkable_height, 18);
    assert_eq!(v.walkable_climb, 4);
}

#[test]
fn a_tiny_agent_never_gets_zero_cells() {
    let v = voxel_config(&AgentParams {
        radius: 0.01,
        height: 0.05,
        max_step: 0.0,
        max_slope_deg: 45.0,
    });
    assert!(v.walkable_radius >= 1);
    assert!(v.walkable_height >= 1);
}

// ---------------------------------------------------------------- baking

fn add_box(g: &mut NavGeometry, min: [f32; 3], max: [f32; 3]) {
    let base = g.vertices.len() as u32;
    for &y in &[min[1], max[1]] {
        for &(x, z) in &[
            (min[0], min[2]),
            (max[0], min[2]),
            (max[0], max[2]),
            (min[0], max[2]),
        ] {
            g.vertices.push([x, y, z]);
        }
    }
    // Both windings, so the box is solid whichever way the baker orients triangles.
    let quads = [
        [0u32, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    for q in quads {
        for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
            g.triangles.push([base + tri[0], base + tri[1], base + tri[2]]);
            g.areas.push(NavAreaId::WALKABLE);
            g.triangles.push([base + tri[0], base + tri[2], base + tri[1]]);
            g.areas.push(NavAreaId::WALKABLE);
        }
    }
}

/// A flat floor of `size` x `size` metres, thin, with its top at y = 0.
fn floor(size: f32) -> NavGeometry {
    let mut g = NavGeometry::default();
    add_box(&mut g, [0.0, -0.2, 0.0], [size, 0.0, size]);
    g
}

fn bake_world(g: &NavGeometry, tile: f32) -> (NavData, NavWorld) {
    let d = bake(g, &AgentParams::default(), tile).unwrap();
    let w = NavWorld::from_data(&d).unwrap();
    (d, w)
}

#[test]
fn a_flat_floor_is_covered_and_connected() {
    let (d, w) = bake_world(&floor(10.0), 16.0);
    assert!(!d.tiles.is_empty());
    assert!(w.find_path(p(2.0, 2.0), p(8.0, 8.0), AgentClass(0)).is_some());
}

#[test]
fn a_wall_with_a_doorway_is_passable_only_through_the_doorway() {
    let mut g = floor(12.0);
    // Wall along x at z = 6 with a doorway 1.6 m wide at x in [5.2, 6.8].
    add_box(&mut g, [0.0, 0.0, 5.8], [5.2, 2.5, 6.2]);
    add_box(&mut g, [6.8, 0.0, 5.8], [12.0, 2.5, 6.2]);
    let (_, w) = bake_world(&g, 16.0);
    let path = w
        .find_path(p(2.0, 2.0), p(2.0, 10.0), AgentClass(0))
        .expect("the doorway connects both sides");
    let through = path
        .iter()
        .any(|q| (q.z - 6.0).abs() < 0.6 && q.x > 5.0 && q.x < 7.0);
    assert!(through, "path {path:?} does not use the doorway");
    assert!(path_length(&path) > 8.0, "longer than the straight line");
}

#[test]
fn an_obstacle_splits_the_coverage() {
    let mut g = floor(12.0);
    add_box(&mut g, [0.0, 0.0, 5.8], [12.0, 2.5, 6.2]);
    let (_, w) = bake_world(&g, 16.0);
    assert!(w.find_path(p(2.0, 2.0), p(2.0, 10.0), AgentClass(0)).is_none());
}

#[test]
fn a_not_walkable_area_is_excluded() {
    let mut g = floor(10.0);
    for area in &mut g.areas {
        *area = NavAreaId::NOT_WALKABLE;
    }
    match bake(&g, &AgentParams::default(), 16.0) {
        Ok(d) => assert!(d.tiles.iter().all(|t| t.polygons.is_empty())),
        Err(_) => {}
    }
}

#[test]
fn baking_twice_is_bit_identical() {
    let g = floor(10.0);
    let a = bake(&g, &AgentParams::default(), 16.0).unwrap().to_bytes();
    let b = bake(&g, &AgentParams::default(), 16.0).unwrap().to_bytes();
    assert_eq!(a, b);
    let g2 = g.clone();
    let c = std::thread::spawn(move || {
        bake(&g2, &AgentParams::default(), 16.0).unwrap().to_bytes()
    })
    .join()
    .unwrap();
    assert_eq!(a, c);
}

#[test]
fn empty_geometry_is_an_error() {
    assert_eq!(
        bake(&NavGeometry::default(), &AgentParams::default(), 16.0).unwrap_err(),
        BakeError::EmptyGeometry
    );
}

#[test]
fn a_bad_tile_size_is_an_error() {
    for size in [0.0, -1.0, f32::NAN] {
        assert_eq!(
            bake(&floor(4.0), &AgentParams::default(), size).unwrap_err(),
            BakeError::BadTileSize
        );
    }
}

#[test]
fn neighbouring_tiles_are_connected() {
    // 24 m floor with 8 m tiles: three tiles per side.
    let (d, w) = bake_world(&floor(24.0), 8.0);
    assert!(d.tiles.len() >= 4, "tiles: {}", d.tiles.len());
    assert!(w.find_path(p(2.0, 2.0), p(22.0, 22.0), AgentClass(0)).is_some());
}

// ---------------------------------------------------------------- paths

#[test]
fn a_straight_corridor_gives_a_short_path() {
    let w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 10, 1, 1.0, walk)]);
    let path = w.find_path(p(0.5, 0.5), p(9.5, 0.5), AgentClass(0)).unwrap();
    assert!(path.len() >= 2);
    assert!((path_length(&path) - 9.0).abs() < 0.2, "{path:?}");
}

#[test]
fn a_path_goes_around_a_wall() {
    // 6 x 5 cells, with the column x = 2 missing for z in 0..4: the gap is at z = 4.
    let w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 6, 5, 1.0, |c, r| {
        if c == 2 && r < 4 { None } else { Some(NavAreaId::WALKABLE) }
    })]);
    let path = w.find_path(p(0.5, 0.5), p(5.5, 0.5), AgentClass(0)).unwrap();
    assert!(path_length(&path) > 6.0, "longer than straight: {path:?}");
    for q in &path {
        assert!(w.nearest_point(*q, 0.05).is_some(), "{q:?} is off the mesh");
    }
}

#[test]
fn an_unreachable_target_gives_none() {
    let w = world_of(vec![
        grid_tile((0, 0), 0.0, 0.0, 3, 3, 1.0, walk),
        grid_tile((1, 0), 10.0, 0.0, 3, 3, 1.0, walk),
    ]);
    assert!(w.find_path(p(1.0, 1.0), p(11.0, 1.0), AgentClass(0)).is_none());
}

#[test]
fn a_path_crosses_a_tile_border() {
    let w = world_of(vec![
        grid_tile((0, 0), 0.0, 0.0, 4, 2, 1.0, walk),
        grid_tile((1, 0), 4.0, 0.0, 4, 2, 1.0, walk),
    ]);
    let path = w.find_path(p(0.5, 1.0), p(7.5, 1.0), AgentClass(0)).unwrap();
    assert!((path_length(&path) - 7.0).abs() < 0.3, "{path:?}");
}

/// 5 x 3 cells; the middle row (z in 1..2) is swamp (area 1) for x in 1..4.
fn swamp_world() -> NavWorld {
    world_of(vec![grid_tile((0, 0), 0.0, 0.0, 5, 3, 1.0, |c, r| {
        if r == 1 && (1..4).contains(&c) {
            Some(NavAreaId(1))
        } else {
            Some(NavAreaId::WALKABLE)
        }
    })])
}

#[test]
fn area_cost_changes_the_route() {
    let mut w = swamp_world();
    let from = p(0.5, 1.5);
    let to = p(4.5, 1.5);
    let cheap = w.find_path(from, to, AgentClass(0)).unwrap();
    assert!(cheap.iter().all(|q| (q.z - 1.5).abs() < 0.05), "{cheap:?}");
    w.set_area_cost(NavAreaId(1), 10.0).unwrap();
    let detour = w.find_path(from, to, AgentClass(0)).unwrap();
    assert!(
        detour.iter().any(|q| (q.z - 1.5).abs() > 0.4),
        "no detour: {detour:?}"
    );
}

#[test]
fn infinite_cost_blocks_the_area() {
    // A one-cell-wide corridor whose middle cell is swamp.
    let mut w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 5, 1, 1.0, |c, _| {
        if c == 2 { Some(NavAreaId(1)) } else { Some(NavAreaId::WALKABLE) }
    })]);
    w.set_area_cost(NavAreaId(1), f32::INFINITY).unwrap();
    assert!(w.find_path(p(0.5, 0.5), p(4.5, 0.5), AgentClass(0)).is_none());
}

#[test]
fn bad_costs_are_rejected() {
    let mut w = swamp_world();
    assert_eq!(w.set_area_cost(NavAreaId(1), f32::NAN), Err(NavError::BadCost));
    assert_eq!(w.set_area_cost(NavAreaId(1), -1.0), Err(NavError::BadCost));
    assert_eq!(w.set_area_cost(NavAreaId(1), 0.0), Err(NavError::BadCost));
}

#[test]
fn replacing_a_tile_changes_the_answer() {
    let left = grid_tile((0, 0), 0.0, 0.0, 3, 2, 1.0, walk);
    let right = grid_tile((1, 0), 3.0, 0.0, 3, 2, 1.0, walk);
    let mut w = world_of(vec![left, right.clone()]);
    assert!(w.find_path(p(0.5, 1.0), p(5.5, 1.0), AgentClass(0)).is_some());
    assert!(w.remove_tile((1, 0)));
    assert!(!w.remove_tile((1, 0)), "removed twice");
    assert!(w.find_path(p(0.5, 1.0), p(5.5, 1.0), AgentClass(0)).is_none());
    w.replace_tile(right).unwrap();
    assert!(w.find_path(p(0.5, 1.0), p(5.5, 1.0), AgentClass(0)).is_some());
}

#[test]
fn nearest_point_snaps_to_the_mesh_and_respects_the_limit() {
    let w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 4, 4, 1.0, walk)]);
    let snapped = w.nearest_point(Vec3::new(2.0, 0.3, 2.0), 1.0).unwrap();
    assert!(snapped.y.abs() < 0.05, "{snapped:?}");
    assert!(w.nearest_point(p(50.0, 50.0), 1.0).is_none());
    assert!(w.nearest_point(p(2.0, 2.0), f32::NAN).is_none());
}

#[test]
fn raycast_stops_at_the_mesh_edge() {
    let w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 4, 4, 1.0, walk)]);
    let hit = w.raycast(p(2.0, 2.0), p(10.0, 2.0)).unwrap();
    assert!(hit.x > 3.5 && hit.x < 4.3, "{hit:?}");
    assert!(w.raycast(p(1.0, 2.0), p(3.0, 2.0)).is_none(), "the whole segment is on the mesh");
}

#[test]
fn random_points_lie_on_the_mesh() {
    let w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 4, 4, 1.0, walk)]);
    let mut state = 12345u32;
    let mut rng = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    for _ in 0..200 {
        let q = w.random_point_in_radius(p(2.0, 2.0), 1.5, &mut rng).unwrap();
        assert!(w.nearest_point(q, 0.05).is_some(), "{q:?} off the mesh");
        assert!((q - p(2.0, 2.0)).length() < 2.5);
    }
}

// ---------------------------------------------------------------- links

/// Two platforms with a 2 m gap between them (x in 3..5).
fn gap_world() -> NavWorld {
    world_of(vec![
        grid_tile((0, 0), 0.0, 0.0, 3, 3, 1.0, walk),
        grid_tile((1, 0), 5.0, 0.0, 3, 3, 1.0, walk),
    ])
}

fn jump(bidirectional: bool, class_mask: u32) -> NavLink {
    NavLink {
        from: p(2.8, 1.5),
        to: p(5.2, 1.5),
        bidirectional,
        class_mask,
        cost: 1.0,
    }
}

#[test]
fn a_link_lets_the_path_jump_a_gap() {
    let mut w = gap_world();
    assert!(w.find_path(p(1.0, 1.5), p(7.0, 1.5), AgentClass(0)).is_none());
    w.add_link(jump(true, u32::MAX)).unwrap();
    let path = w.find_path(p(1.0, 1.5), p(7.0, 1.5), AgentClass(0)).unwrap();
    assert!(path.iter().any(|q| q.x > 2.5 && q.x < 3.1), "{path:?}");
    assert!(path.iter().any(|q| q.x > 4.9 && q.x < 5.5), "{path:?}");
}

#[test]
fn a_one_way_link_is_not_used_backwards() {
    let mut w = gap_world();
    w.add_link(jump(false, u32::MAX)).unwrap();
    assert!(w.find_path(p(1.0, 1.5), p(7.0, 1.5), AgentClass(0)).is_some());
    assert!(w.find_path(p(7.0, 1.5), p(1.0, 1.5), AgentClass(0)).is_none());
}

#[test]
fn class_mask_hides_the_link_from_other_classes() {
    let mut w = gap_world();
    w.add_link(jump(true, 0b1)).unwrap();
    assert!(w.find_path(p(1.0, 1.5), p(7.0, 1.5), AgentClass(0)).is_some());
    assert!(w.find_path(p(1.0, 1.5), p(7.0, 1.5), AgentClass(1)).is_none());
}

#[test]
fn a_link_far_from_the_mesh_is_rejected() {
    let mut w = gap_world();
    let mut link = jump(true, u32::MAX);
    link.from = p(2.8, 40.0);
    assert_eq!(w.add_link(link), Err(NavError::LinkOffMesh));
    let mut link = jump(true, u32::MAX);
    link.cost = 0.0;
    assert_eq!(w.add_link(link), Err(NavError::BadCost));
}

#[test]
fn removing_a_link_closes_the_gap_again() {
    let mut w = gap_world();
    let id = w.add_link(jump(true, u32::MAX)).unwrap();
    assert!(w.remove_link(id));
    assert!(!w.remove_link(id));
    assert!(w.find_path(p(1.0, 1.5), p(7.0, 1.5), AgentClass(0)).is_none());
}

// ---------------------------------------------------------------- queue

#[test]
fn the_queue_respects_the_budget_and_keeps_order() {
    let w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 10, 1, 1.0, walk)]);
    let mut ecs = redox_ecs::World::new();
    let mut queue = PathQueue::new();
    let tickets: Vec<_> = (0..20)
        .map(|_| {
            let e = ecs.spawn();
            queue.submit(e, p(0.5, 0.5), p(9.5, 0.5), AgentClass(0))
        })
        .collect();
    let first = queue.run(&w, 16);
    assert_eq!(first.len(), 16);
    assert_eq!(queue.len(), 4);
    let second = queue.run(&w, 16);
    assert_eq!(second.len(), 4);
    let order: Vec<_> = first.iter().chain(&second).map(|(t, _)| *t).collect();
    assert_eq!(order, tickets);
    assert!(first.iter().all(|(_, path)| path.is_some()));
    assert!(queue.is_empty());
}

// ---------------------------------------------------------------- blockers

#[test]
fn a_blocker_cuts_the_route_and_removing_it_restores_it() {
    let mut w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 10, 2, 1.0, walk)]);
    let footprint = [p(4.0, -0.5), p(6.0, -0.5), p(6.0, 2.5), p(4.0, 2.5)];
    let id = w.add_blocker(&footprint, -1.0, 2.0).unwrap();
    assert!(w.find_path(p(0.5, 1.0), p(9.5, 1.0), AgentClass(0)).is_none());
    assert!(w.remove_blocker(id));
    assert!(!w.remove_blocker(id));
    assert!(w.find_path(p(0.5, 1.0), p(9.5, 1.0), AgentClass(0)).is_some());
}

#[test]
fn a_blocker_on_a_wide_floor_makes_a_detour() {
    let mut w = world_of(vec![grid_tile((0, 0), 0.0, 0.0, 12, 8, 1.0, walk)]);
    let footprint = [p(5.0, 2.0), p(7.0, 2.0), p(7.0, 6.0), p(5.0, 6.0)];
    w.add_blocker(&footprint, -1.0, 2.0).unwrap();
    let path = w.find_path(p(0.5, 4.0), p(11.5, 4.0), AgentClass(0)).unwrap();
    assert!(path_length(&path) > 11.5, "{path:?}");
    for q in &path {
        let inside = q.x > 5.05 && q.x < 6.95 && q.z > 2.05 && q.z < 5.95;
        assert!(!inside, "{q:?} is inside the blocker");
    }
}

#[test]
fn bad_blockers_are_rejected() {
    let mut w = swamp_world();
    assert_eq!(
        w.add_blocker(&[p(0.0, 0.0), p(1.0, 0.0)], 0.0, 1.0),
        Err(NavError::BadBlocker)
    );
    let tri = [p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)];
    assert_eq!(w.add_blocker(&tri, 2.0, 1.0), Err(NavError::BadBlocker));
}

// ---------------------------------------------------------------- avoidance

fn open_floor() -> NavWorld {
    world_of(vec![grid_tile((0, 0), 0.0, 0.0, 30, 12, 1.0, walk)])
}

/// Integrates agents with the desired velocity of the world for `seconds`.
fn simulate(world: &mut NavWorld, agents: &mut [(redox_nav::NavAgentId, Vec3, Vec3)], seconds: f32) {
    let dt = 1.0 / 60.0;
    for _ in 0..(seconds / dt) as usize {
        let desired = world.update_agents(dt, agents);
        for (id, velocity) in desired {
            if let Some(a) = agents.iter_mut().find(|a| a.0 == id) {
                a.2 = velocity;
                a.1 += velocity * dt;
            }
        }
    }
}

#[test]
fn two_agents_head_on_pass_each_other() {
    let mut w = open_floor();
    let a = w
        .add_agent(p(3.0, 6.0), 0.4, 2.0, AvoidanceParams::default())
        .unwrap();
    let b = w
        .add_agent(p(27.0, 6.0), 0.4, 2.0, AvoidanceParams::default())
        .unwrap();
    w.set_goal(a, Some(p(27.0, 6.0)));
    w.set_goal(b, Some(p(3.0, 6.0)));
    let mut agents = vec![(a, p(3.0, 6.0), Vec3::ZERO), (b, p(27.0, 6.0), Vec3::ZERO)];
    let mut closest = f32::MAX;
    let dt = 1.0 / 60.0;
    for _ in 0..(20.0 / dt) as usize {
        simulate(&mut w, &mut agents, dt);
        closest = closest.min((agents[0].1 - agents[1].1).length());
    }
    assert!(closest > 0.8 * 0.72, "agents overlapped: closest {closest}");
    assert!((agents[0].1 - p(27.0, 6.0)).length() < 1.5, "a at {:?}", agents[0].1);
    assert!((agents[1].1 - p(3.0, 6.0)).length() < 1.5, "b at {:?}", agents[1].1);
}

#[test]
fn a_lone_agent_walks_straight_to_its_goal() {
    let mut w = open_floor();
    let a = w
        .add_agent(p(3.0, 6.0), 0.4, 2.0, AvoidanceParams::default())
        .unwrap();
    w.set_goal(a, Some(p(23.0, 6.0)));
    let mut agents = vec![(a, p(3.0, 6.0), Vec3::ZERO)];
    simulate(&mut w, &mut agents, 12.0);
    assert!((agents[0].1 - p(23.0, 6.0)).length() < 1.0, "{:?}", agents[0].1);
}

#[test]
fn an_agent_without_a_goal_stays_put() {
    let mut w = open_floor();
    let a = w
        .add_agent(p(5.0, 6.0), 0.4, 2.0, AvoidanceParams::default())
        .unwrap();
    let mut agents = vec![(a, p(5.0, 6.0), Vec3::ZERO)];
    simulate(&mut w, &mut agents, 2.0);
    assert!((agents[0].1 - p(5.0, 6.0)).length() < 0.1);
}

#[test]
fn removing_an_agent_forgets_it() {
    let mut w = open_floor();
    let a = w
        .add_agent(p(5.0, 6.0), 0.4, 2.0, AvoidanceParams::default())
        .unwrap();
    assert_eq!(w.agent_count(), 1);
    assert!(w.remove_agent(a));
    assert!(!w.remove_agent(a));
    assert_eq!(w.agent_count(), 0);
    assert!(w.update_agents(0.016, &[(a, p(5.0, 6.0), Vec3::ZERO)]).is_empty());
}

#[test]
fn bad_agents_are_rejected() {
    let mut w = open_floor();
    for (radius, speed) in [(0.0, 1.0), (0.4, 0.0), (f32::NAN, 1.0)] {
        assert_eq!(
            w.add_agent(p(5.0, 6.0), radius, speed, AvoidanceParams::default())
                .unwrap_err(),
            NavError::BadAgent
        );
    }
}

#[test]
fn a_crowd_of_a_hundred_reaches_the_goal_without_overlapping_deeply() {
    let mut w = open_floor();
    let mut agents = Vec::new();
    for i in 0..100u16 {
        let start = p(2.0 + f32::from(i % 10) * 1.0, 1.5 + f32::from(i / 10));
        let id = w
            .add_agent(start, 0.3, 2.0, AvoidanceParams::default())
            .unwrap();
        w.set_goal(id, Some(p(26.0 + f32::from(i % 10) * 0.2, 6.0)));
        agents.push((id, start, Vec3::ZERO));
    }
    let mut deepest: f32 = 0.0;
    let dt = 1.0 / 60.0;
    for _ in 0..(40.0 / dt) as usize {
        simulate(&mut w, &mut agents, dt);
        for i in 0..agents.len() {
            for j in (i + 1)..agents.len() {
                let d = (agents[i].1 - agents[j].1).length();
                deepest = deepest.max((0.6 - d).max(0.0) / 0.6);
            }
        }
    }
    assert!(deepest < 0.5, "overlap {deepest}");
    let arrived = agents
        .iter()
        .filter(|a| a.1.x > 23.0)
        .count();
    assert!(arrived >= 90, "only {arrived} of 100 arrived");
}

#[test]
#[ignore = "timing: run with --release -- --ignored --nocapture"]
fn avoidance_bench_two_hundred_agents() {
    let mut w = open_floor();
    let mut agents = Vec::new();
    for i in 0..200u16 {
        let start = p(1.5 + f32::from(i % 20) * 0.8, 1.0 + f32::from(i / 20));
        let id = w.add_agent(start, 0.3, 2.0, AvoidanceParams::default()).unwrap();
        w.set_goal(id, Some(p(27.0, 6.0)));
        agents.push((id, start, Vec3::ZERO));
    }
    let started = std::time::Instant::now();
    let frames = 120;
    for _ in 0..frames {
        let desired = w.update_agents(1.0 / 60.0, &agents);
        for (id, v) in desired {
            if let Some(a) = agents.iter_mut().find(|a| a.0 == id) {
                a.2 = v;
                a.1 += v / 60.0;
            }
        }
    }
    let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    eprintln!("200 agents: {per_frame:.3} ms per update");
}

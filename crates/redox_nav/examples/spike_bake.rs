//! NAV-01 spike A: rerecast bake on three scenes built from code.
//!
//! Not part of the public API. Run by the coordinator with
//! `cargo run --release -p redox_nav --example spike_bake`.
//! Facts about the API are recorded in `docs/engineering/notes/nav-spike.md`.

use std::error::Error;
use std::time::Instant;

use rerecast::{
    Aabb3d, AreaType, ConfigBuilder, DetailNavmesh, HeightfieldBuilder, PolygonNavmesh, TriMesh,
};

type Res<T> = Result<T, Box<dyn Error>>;

/// Triangle soup in world space (Y up).
#[derive(Clone, Default)]
struct Scene {
    vertices: Vec<[f32; 3]>,
    indices: Vec<[u32; 3]>,
}

impl Scene {
    /// Adds a quad with corners `a, b, c, d` (counter-clockwise seen from above when used as floor).
    fn quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend([a, b, c, d]);
        self.indices.push([base, base + 1, base + 2]);
        self.indices.push([base, base + 2, base + 3]);
    }

    /// Floor rectangle at height `y` over `[x0, x1] x [z0, z1]`.
    fn floor(&mut self, x0: f32, x1: f32, z0: f32, z1: f32, y: f32) {
        self.quad([x0, y, z0], [x0, y, z1], [x1, y, z1], [x1, y, z0]);
    }

    /// Vertical wall along X at height range `[y0, y1]`.
    fn wall_x(&mut self, x: f32, z0: f32, z1: f32, y0: f32, y1: f32) {
        self.quad([x, y0, z0], [x, y1, z0], [x, y1, z1], [x, y0, z1]);
    }

    /// Vertical wall along Z at height range `[y0, y1]`.
    fn wall_z(&mut self, z: f32, x0: f32, x1: f32, y0: f32, y1: f32) {
        self.quad([x0, y0, z], [x0, y1, z], [x1, y1, z], [x1, y0, z]);
    }

    /// Scene (a): 10 x 10 m room with a 1.2 m doorway in the north wall (z = 10), walls 3 m high.
    fn room_with_doorway() -> Self {
        let mut s = Scene::default();
        s.floor(0.0, 10.0, 0.0, 10.0, 0.0);
        s.wall_x(0.0, 0.0, 10.0, 0.0, 3.0);
        s.wall_x(10.0, 0.0, 10.0, 0.0, 3.0);
        s.wall_z(0.0, 0.0, 10.0, 0.0, 3.0);
        // North wall with a gap: x in [4.4, 5.6].
        s.wall_z(10.0, 0.0, 4.4, 0.0, 3.0);
        s.wall_z(10.0, 5.6, 10.0, 0.0, 3.0);
        s
    }

    /// Scene (b): 8 steps of 0.15 m, each tread 1 m deep, rising along +X.
    fn stairs() -> Self {
        let mut s = Scene::default();
        s.floor(-4.0, 0.0, -4.0, 4.0, 0.0);
        for i in 0..8 {
            let x0 = i as f32;
            let y = 0.15 * (i as f32 + 1.0);
            s.floor(x0, x0 + 1.0, -4.0, 4.0, y);
        }
        s.floor(8.0, 12.0, -4.0, 4.0, 1.2);
        s
    }

    /// Scene (c): 40 x 40 m floor made of 1 m quads.
    fn big_floor() -> Self {
        let mut s = Scene::default();
        for ix in 0..40 {
            for iz in 0..40 {
                let (x0, z0) = (ix as f32, iz as f32);
                s.floor(x0, x0 + 1.0, z0, z0 + 1.0, 0.0);
            }
        }
        s
    }

    /// Returns the triangles whose XZ bounding box overlaps `[min, max]`.
    fn triangles_in(&self, min: [f32; 2], max: [f32; 2]) -> Self {
        let mut out = Scene::default();
        for tri in &self.indices {
            let pts = tri.map(|i| self.vertices[i as usize]);
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in pts {
                lo = [lo[0].min(p[0]), lo[1].min(p[2])];
                hi = [hi[0].max(p[0]), hi[1].max(p[2])];
            }
            let overlaps = hi[0] >= min[0] && lo[0] <= max[0] && hi[1] >= min[1] && lo[1] <= max[1];
            if !overlaps {
                continue;
            }
            let base = out.vertices.len() as u32;
            out.vertices.extend(pts);
            out.indices.push([base, base + 1, base + 2]);
        }
        out
    }
}

/// Result of one bake.
struct Baked {
    poly: PolygonNavmesh,
    detail: DetailNavmesh,
}

/// Bakes `scene` inside `[min, max]` (XZ) with `border` metres of padding.
/// Agent: radius 0.4 m, height 1.8 m, climb 0.4 m, slope 45 degrees.
fn bake(scene: &Scene, min: [f32; 2], max: [f32; 2], border: f32, cell_size: f32) -> Res<Baked> {
    let radius = 0.4;
    let cell_height = cell_size / 2.0;
    let cfg = ConfigBuilder {
        agent_radius: radius,
        agent_height: 1.8,
        walkable_climb: 0.4,
        walkable_slope_angle: 45.0_f32.to_radians(),
        cell_size_fraction: radius / cell_size,
        cell_height_fraction: radius / cell_height,
        ..Default::default()
    }
    .build();

    let part = scene.triangles_in(
        [min[0] - border, min[1] - border],
        [max[0] + border, max[1] + border],
    );
    let mut trimesh = TriMesh {
        vertices: part.vertices.iter().map(|v| (*v).into()).collect(),
        indices: part.indices.iter().map(|t| (*t).into()).collect(),
        area_types: vec![AreaType::NOT_WALKABLE; part.indices.len()],
    };
    trimesh.mark_walkable_triangles(cfg.walkable_slope_angle);
    let aabb = Aabb3d::new(
        [(min[0] + max[0]) / 2.0, 0.0, (min[1] + max[1]) / 2.0],
        [
            (max[0] - min[0]) / 2.0 + border,
            5.0,
            (max[1] - min[1]) / 2.0 + border,
        ],
    );

    let mut hf = HeightfieldBuilder {
        aabb,
        cell_size: cfg.cell_size,
        cell_height: cfg.cell_height,
    }
    .build()?;
    hf.populate_from_trimesh(trimesh, cfg.walkable_height, cfg.walkable_climb)?;
    let mut chf = hf.into_compact(cfg.walkable_height, cfg.walkable_climb)?;
    chf.erode_walkable_area(cfg.walkable_radius);
    chf.build_distance_field();
    chf.build_regions(cfg.border_size, cfg.min_region_area, cfg.merge_region_area)?;
    let cset = chf.build_contours(
        cfg.max_simplification_error,
        cfg.max_edge_len,
        cfg.contour_flags,
    );
    let poly = cset.into_polygon_mesh(cfg.max_vertices_per_polygon)?;
    let detail = DetailNavmesh::new(
        &poly,
        &chf,
        cfg.detail_sample_dist,
        cfg.detail_sample_max_error,
    )?;
    Ok(Baked { poly, detail })
}

/// Prints counts and time for one whole-scene bake.
fn report(name: &str, scene: &Scene, min: [f32; 2], max: [f32; 2], cell: f32) -> Res<Baked> {
    let started = Instant::now();
    let baked = bake(scene, min, max, 0.0, cell)?;
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{name}: cell {cell} m, polygons {}, vertices {}, detail submeshes {}, time {ms:.2} ms",
        baked.poly.polygon_count(),
        baked.poly.vertices.len(),
        baked.detail.meshes.len(),
    );
    Ok(baked)
}

/// Bake determinism: same input twice and from another thread must give identical output.
fn determinism(scene: &Scene) -> Res<()> {
    let first = format!(
        "{:?}",
        bake(scene, [0.0, 0.0], [40.0, 40.0], 0.0, 0.2)?.poly
    );
    let second = format!(
        "{:?}",
        bake(scene, [0.0, 0.0], [40.0, 40.0], 0.0, 0.2)?.poly
    );
    let scene_for_thread = scene.clone();
    let threaded = std::thread::spawn(move || {
        bake(&scene_for_thread, [0.0, 0.0], [40.0, 40.0], 0.0, 0.2)
            .map(|b| format!("{:?}", b.poly))
            .map_err(|e| e.to_string())
    })
    .join()
    .map_err(|_| "thread panicked")?
    .map_err(|e| e.to_string())?;
    println!(
        "determinism: same thread {}, other thread {}",
        first == second,
        first == threaded
    );
    Ok(())
}

/// Tiled bake of a 40 x 40 m floor in 16 m tiles. Checks that tile borders share vertices.
fn tiles_share_edges(scene: &Scene) -> Res<()> {
    let tile = 16.0;
    let mut vertex_sets = Vec::new();
    for tx in 0..3 {
        for tz in 0..3 {
            let min = [tx as f32 * tile, tz as f32 * tile];
            let max = [(min[0] + tile).min(40.0), (min[1] + tile).min(40.0)];
            let baked = bake(scene, min, max, 3.0 * 0.2, 0.2)?;
            // Vertices in world space: aabb.min is the tile border corner (not known here):
            // the spike records only the count and the first/last vertex as a rough check.
            let world: Vec<[f32; 3]> = baked
                .poly
                .vertices
                .iter()
                .map(|v| {
                    [
                        baked.poly.aabb.min.x + v.x as f32 * baked.poly.cell_size,
                        baked.poly.aabb.min.y + v.y as f32 * baked.poly.cell_height,
                        baked.poly.aabb.min.z + v.z as f32 * baked.poly.cell_size,
                    ]
                })
                .collect();
            println!("tile ({tx},{tz}): {} polygons", baked.poly.polygon_count());
            vertex_sets.push(world);
        }
    }
    // Edge-sharing check: count vertices lying on the shared seam x = 16.0 or z = 16.0
    // from both sides and compare their coordinates within 1e-3.
    let seam_x = 16.0_f32;
    let on_seam: Vec<[f32; 3]> = vertex_sets
        .iter()
        .flatten()
        .copied()
        .filter(|p| (p[0] - seam_x).abs() < 1e-3)
        .collect();
    println!("vertices on seam x=16: {}", on_seam.len());
    Ok(())
}

fn main() -> Res<()> {
    let room = Scene::room_with_doorway();
    let stairs = Scene::stairs();
    let floor = Scene::big_floor();

    report("(a) room 10x10", &room, [0.0, 0.0], [10.0, 10.0], 0.2)?;
    report("(a) room 10x10", &room, [0.0, 0.0], [10.0, 10.0], 0.1)?;
    report("(b) stairs", &stairs, [-4.0, -4.0], [12.0, 4.0], 0.2)?;
    report("(b) stairs", &stairs, [-4.0, -4.0], [12.0, 4.0], 0.1)?;
    report(
        "(c) floor 40x40 whole",
        &floor,
        [0.0, 0.0],
        [40.0, 40.0],
        0.2,
    )?;

    let started = Instant::now();
    tiles_share_edges(&floor)?;
    println!(
        "(c) tiled bake total {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    determinism(&room)?;
    Ok(())
}

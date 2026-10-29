//! Queries on a [`NavWorld`]: paths, nearest point, walking raycast, random points.

use redox_math::Vec3;

use crate::backend;
use crate::data::{NavPolygon, NavTile};
use crate::world::{AgentClass, NavWorld};

/// How close a point must be to the mesh to count as on it, in metres.
const ON_MESH_TOLERANCE: f32 = 1e-3;
/// Smallest step of [`NavWorld::raycast`], in metres.
const MIN_RAY_STEP: f32 = 0.01;
/// Halvings of the interval that locate the mesh edge in [`NavWorld::raycast`].
const RAY_REFINEMENT_STEPS: u32 = 16;
/// Attempts of [`NavWorld::random_point_in_radius`] before it gives up.
const RANDOM_ATTEMPTS: u32 = 16;

impl NavWorld {
    /// Shortest path from `from` to `to` for an agent of `class`, as waypoints.
    ///
    /// Both ends are snapped to the mesh, so the first and last waypoints may differ from the
    /// inputs. Animation links contribute their start and end points.
    /// Returns `None` when a point is off the mesh or the target is unreachable.
    pub fn find_path(&self, from: Vec3, to: Vec3, class: AgentClass) -> Option<Vec<Vec3>> {
        let distance = &self.archipelago.archipelago_options.point_sample_distance;
        let start = self
            .archipelago
            .sample_point(backend::to_lm(from), distance)
            .ok()?;
        let end = self
            .archipelago
            .sample_point(backend::to_lm(to), distance)
            .ok()?;
        let permitted = backend::permitted(self.permitted_kinds(class));
        backend::find_path(&self.archipelago, &start, &end, permitted).ok()
    }

    /// Nearest point of the mesh within `max_distance` of `p`, or `None`.
    pub fn nearest_point(&self, p: Vec3, max_distance: f32) -> Option<Vec3> {
        if !(max_distance.is_finite() && max_distance >= 0.0) {
            return None;
        }
        let distance = backend::sample_distance(max_distance, max_distance);
        self.archipelago
            .sample_point(backend::to_lm(p), &distance)
            .ok()
            .map(|sampled| backend::from_lm(sampled.point()))
    }

    /// Walks the straight segment `from -> to` along the mesh.
    ///
    /// Returns the last point that is still on the mesh before the segment leaves it (located to
    /// about `length / 65536`), or `None` when the whole segment lies on the mesh. A start that
    /// is not on the mesh is returned as is.
    pub fn raycast(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        if !self.on_mesh(from) {
            return Some(from);
        }
        let length = (to - from).length();
        if length <= 0.0 {
            return None;
        }
        let step = (self.agent.radius * 0.25).max(MIN_RAY_STEP);
        let count = (length / step).ceil().max(1.0) as u32;
        let mut last_on = 0.0_f32;
        for index in 1..=count {
            let t = index as f32 / count as f32;
            if self.on_mesh(from.lerp(to, t)) {
                last_on = t;
                continue;
            }
            let (mut inside, mut outside) = (last_on, t);
            for _ in 0..RAY_REFINEMENT_STEPS {
                let middle = (inside + outside) / 2.0;
                if self.on_mesh(from.lerp(to, middle)) {
                    inside = middle;
                } else {
                    outside = middle;
                }
            }
            return Some(from.lerp(to, inside));
        }
        None
    }

    /// Random point on the mesh within `radius` of `center`, or `None`.
    ///
    /// Polygons are chosen in proportion to their area; the point is uniform in a fan triangle
    /// of the polygon (an approximation for non-triangular polygons). `rng` returns numbers in
    /// `[0, 1)`. Polygons of blocked areas are not used.
    pub fn random_point_in_radius(
        &self,
        center: Vec3,
        radius: f32,
        rng: &mut impl FnMut() -> f32,
    ) -> Option<Vec3> {
        let candidates = self.candidates_near(center, radius);
        let total: f32 = candidates.iter().map(|(_, _, area)| *area).sum();
        if candidates.is_empty() || !(total > 0.0) {
            return None;
        }
        for _ in 0..RANDOM_ATTEMPTS {
            let mut pick = unit(rng) * total;
            let chosen = candidates
                .iter()
                .find(|(_, _, area)| {
                    if pick < *area {
                        true
                    } else {
                        pick -= *area;
                        false
                    }
                })
                .or(candidates.last());
            let Some((tile, polygon, _)) = chosen else {
                return None;
            };
            let point = sample_polygon(tile, polygon, rng)?;
            if xz_distance(point, center) <= radius {
                return Some(point);
            }
        }
        None
    }

    fn on_mesh(&self, p: Vec3) -> bool {
        self.nearest_point(p, ON_MESH_TOLERANCE).is_some()
    }

    /// Polygons (with their area) that have a corner or a centre within `radius` of `center`.
    fn candidates_near(&self, center: Vec3, radius: f32) -> Vec<(&NavTile, &NavPolygon, f32)> {
        let mut out = Vec::new();
        for tile in self.tiles.values() {
            for polygon in &tile.polygons {
                if self.blocked.contains(&polygon.area) {
                    continue;
                }
                let corners = corners_of(tile, polygon);
                if corners.len() < 3 {
                    continue;
                }
                let near = corners
                    .iter()
                    .any(|corner| xz_distance(*corner, center) <= radius)
                    || xz_distance(centroid(&corners), center) <= radius;
                if !near {
                    continue;
                }
                let area = fan_areas(&corners).iter().sum::<f32>();
                if area > 0.0 {
                    out.push((tile, polygon, area));
                }
            }
        }
        out
    }
}

/// Corners of a polygon in world space; indices outside the tile are skipped.
fn corners_of(tile: &NavTile, polygon: &NavPolygon) -> Vec<Vec3> {
    polygon
        .indices
        .iter()
        .filter_map(|&i| tile.vertices.get(usize::from(i)))
        .map(|v| Vec3::from_array(*v))
        .collect()
}

fn centroid(corners: &[Vec3]) -> Vec3 {
    corners.iter().copied().sum::<Vec3>() / corners.len() as f32
}

/// Horizontal (XZ) area of each fan triangle of a convex polygon.
fn fan_areas(corners: &[Vec3]) -> Vec<f32> {
    let Some(&origin) = corners.first() else {
        return Vec::new();
    };
    (1..corners.len().saturating_sub(1))
        .map(|i| {
            let a = corners[i] - origin;
            let b = corners[i + 1] - origin;
            0.5 * (a.x * b.z - a.z * b.x).abs()
        })
        .collect()
}

/// Uniform point in one fan triangle of the polygon (chosen uniformly among the fan).
fn sample_polygon(
    tile: &NavTile,
    polygon: &NavPolygon,
    rng: &mut impl FnMut() -> f32,
) -> Option<Vec3> {
    let corners = corners_of(tile, polygon);
    if corners.len() < 3 {
        return None;
    }
    let triangles = corners.len() - 2;
    let index = ((unit(rng) * triangles as f32) as usize).min(triangles - 1);
    let (a, b, c) = (corners[0], corners[index + 1], corners[index + 2]);
    let (mut u, mut v) = (unit(rng), unit(rng));
    if u + v > 1.0 {
        u = 1.0 - u;
        v = 1.0 - v;
    }
    Some(a + (b - a) * u + (c - a) * v)
}

/// Draws a number and keeps it in `[0, 1)`.
fn unit(rng: &mut impl FnMut() -> f32) -> f32 {
    rng().clamp(0.0, 0.999_999)
}

fn xz_distance(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

//! Skinned meshes: the pose is applied on the CPU and the deformed vertices are streamed into
//! an ordinary mesh slot, so every existing pass (PBR, VSM / point shadows, normal
//! prepass, GTAO, TAA) draws it unchanged.

use redox_anim::SkinnedGeometry;
use redox_math::{Aabb, Mat4, Vec3};

use crate::mesh::{Mesh, Vertex};
use crate::renderer::Renderer;

/// Default tangent for skinned vertices (normal maps are not skinned yet).
const DEFAULT_TANGENT: [f32; 3] = [1.0, 0.0, 0.0];

/// A skinned mesh uploaded to the renderer plus the scratch buffers to refresh it each frame.
pub struct SkinnedMeshBinding {
    geometry: SkinnedGeometry,
    /// Mesh slot to put into [`crate::RenderObject::mesh_index`].
    pub mesh_slot: usize,
    /// Local-space bounds the vertices are quantised against (also valid for culling).
    pub bounds: Aabb,
    vertices: Vec<Vertex>,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    /// Bounds of the vertices before the most recent [`Self::update`].
    previous_bounds: Option<Aabb>,
}

impl SkinnedMeshBinding {
    /// Uploads the bind pose. `bounds_margin` inflates the bind-pose bounds on every side; it
    /// must cover how far any animation can move a vertex away from the bind pose.
    pub fn new(renderer: &mut Renderer, geometry: &SkinnedGeometry, bounds_margin: f32) -> Self {
        let vertices = bind_vertices(geometry);
        let bind_bounds = vertices
            .iter()
            .fold(Aabb::empty(), |b, v| b.expand(Vec3::from(v.position)));
        let bounds = Aabb::from_center_size(
            bind_bounds.center(),
            bind_bounds.half_extents() + Vec3::splat(bounds_margin.max(0.0)),
        );
        let mesh = Mesh {
            vertices: vertices.clone(),
            indices: geometry.indices.clone(),
            aabb: bounds,
        };
        let mesh_slot = renderer.upload_mesh(&mesh);
        Self {
            geometry: geometry.clone(),
            mesh_slot,
            bounds,
            vertices,
            positions: Vec::new(),
            normals: Vec::new(),
            previous_bounds: None,
        }
    }

    /// Deforms the mesh with `skin_matrices` (`Pose::skin_matrices`) and uploads the result.
    /// Returns `false` if the renderer rejected the update.
    pub fn update(&mut self, renderer: &Renderer, skin_matrices: &[Mat4]) -> bool {
        self.previous_bounds = (!self.positions.is_empty()).then(|| self.deformed_bounds());
        self.geometry
            .deform_into(skin_matrices, &mut self.positions, &mut self.normals);
        for ((vertex, position), normal) in self
            .vertices
            .iter_mut()
            .zip(&self.positions)
            .zip(&self.normals)
        {
            vertex.position = *position;
            vertex.normal = *normal;
        }
        renderer.update_mesh_vertices(self.mesh_slot, &self.vertices)
    }

    /// Tight local-space bounds of the vertices from the last [`Self::update`] (the bind-pose
    /// bounds before the first update).
    pub fn deformed_bounds(&self) -> Aabb {
        let points: &[[f32; 3]] = if self.positions.is_empty() {
            &self.geometry.positions
        } else {
            &self.positions
        };
        points
            .iter()
            .fold(Aabb::empty(), |b, p| b.expand(Vec3::from(*p)))
    }

    /// Bounds covering where the mesh was before the last update and where it is now.
    ///
    /// Shadow pages must be invalidated over this region: pages at the *old* position still
    /// hold the old shadow, and clearing only the new footprint leaves it behind.
    pub fn swept_bounds(&self) -> Aabb {
        let now = self.deformed_bounds();
        match self.previous_bounds {
            Some(prev) => Aabb {
                min: now.min.min(prev.min),
                max: now.max.max(prev.max),
            },
            None => now,
        }
    }

    /// Whether `point` lies inside the quantisation bounds (deformed vertices outside would clip).
    pub fn bounds_contain(&self, point: Vec3) -> bool {
        self.bounds.contains_point(point)
    }

    /// Largest distance any deformed vertex of the last [`Self::update`] lies outside `bounds`.
    pub fn last_overshoot(&self) -> f32 {
        let (min, max) = (
            self.bounds.center() - self.bounds.half_extents(),
            self.bounds.center() + self.bounds.half_extents(),
        );
        self.positions
            .iter()
            .map(|p| {
                let p = Vec3::from(*p);
                (min - p).max(p - max).max(Vec3::ZERO).max_element()
            })
            .fold(0.0, f32::max)
    }
}

fn bind_vertices(geometry: &SkinnedGeometry) -> Vec<Vertex> {
    (0..geometry.vertex_count())
        .map(|i| Vertex {
            position: geometry.positions[i],
            normal: geometry.normals[i],
            uv: geometry.uvs[i],
            tangent: DEFAULT_TANGENT,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use redox_anim::Pose;
    use redox_anim::procedural::chain_model;

    #[test]
    fn bind_vertices_copy_geometry() {
        let model = chain_model(2, 0.5, 0.1, 6);
        let v = bind_vertices(&model.geometry);
        assert_eq!(v.len(), model.geometry.vertex_count());
        assert_eq!(v[3].position, model.geometry.positions[3]);
        assert_eq!(v[3].tangent, DEFAULT_TANGENT);
    }

    /// The margin used by the demo must keep every animated vertex inside the quantisation box.
    #[test]
    fn demo_animation_stays_inside_inflated_bounds() {
        let model = redox_anim::procedural::demo_rig();
        let vertices = bind_vertices(&model.geometry);
        let bind = vertices
            .iter()
            .fold(Aabb::empty(), |b, v| b.expand(Vec3::from(v.position)));
        let margin = bind.half_extents().max_element();
        let bounds =
            Aabb::from_center_size(bind.center(), bind.half_extents() + Vec3::splat(margin));

        let (mut p, mut n) = (Vec::new(), Vec::new());
        for clip in &model.clips {
            for step in 0..40 {
                let t = clip.duration * step as f32 / 40.0;
                let local = clip.sample(&model.skeleton, t, true);
                let pose = Pose::from_local(&model.skeleton, &local);
                model
                    .geometry
                    .deform_into(&pose.skin_matrices(&model.skeleton), &mut p, &mut n);
                for pos in &p {
                    assert!(
                        bounds.contains_point(Vec3::from(*pos)),
                        "{} t={t}: {pos:?} outside {bounds:?}",
                        clip.name
                    );
                }
            }
        }
    }
}

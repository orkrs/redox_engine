//! Object motion for TAA: which objects moved since the previous frame and where they were.
//!
//! The renderer remembers the model matrices of the last frame. Objects are matched across frames
//! by the ids the caller supplied ([`crate::frame::FrameView::with_object_ids`]); without ids an
//! object is matched by its position in the list, and only when its mesh and material are the same
//! as last frame (a changed list then simply yields "no motion" for the objects that shifted).

use std::collections::HashMap;

use redox_math::Mat4;

use crate::systems::RenderObject;

/// Matrices closer than this (largest element difference) count as "did not move".
const STILL_EPSILON: f32 = 1e-5;

/// Remembered state of the previous frame plus the result for the current one.
#[derive(Default)]
pub(super) struct ObjectMotion {
    by_id: HashMap<u64, Mat4>,
    by_index: Vec<(usize, usize, Mat4)>,
    /// Previous model matrix per object of the current frame (the current one when unknown).
    pub(super) prev_matrices: Vec<[[f32; 4]; 4]>,
    /// `(instance index, mesh slot)` of the objects that moved.
    pub(super) moving: Vec<(u32, usize)>,
}

fn max_difference(a: &Mat4, b: &Mat4) -> f32 {
    a.to_cols_array()
        .iter()
        .zip(b.to_cols_array())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

impl ObjectMotion {
    /// Computes the previous matrices and the moving set for `objects`, then remembers `objects`
    /// for the next frame. `ids`, when given, must have one id per object.
    pub(super) fn update(&mut self, objects: &[RenderObject], ids: Option<&[u64]>) {
        let ids = ids.filter(|ids| ids.len() == objects.len());
        self.prev_matrices.clear();
        self.moving.clear();
        for (i, object) in objects.iter().enumerate() {
            let previous = match ids {
                Some(ids) => self.by_id.get(&ids[i]).copied(),
                None => self
                    .by_index
                    .get(i)
                    .filter(|(mesh, material, _)| {
                        *mesh == object.mesh_index && *material == object.material_index
                    })
                    .map(|(_, _, matrix)| *matrix),
            }
            .unwrap_or(object.model_matrix);
            if max_difference(&previous, &object.model_matrix) > STILL_EPSILON {
                self.moving.push((i as u32, object.mesh_index));
            }
            self.prev_matrices.push(previous.to_cols_array_2d());
        }

        self.by_id.clear();
        self.by_index.clear();
        for (i, object) in objects.iter().enumerate() {
            if let Some(ids) = ids {
                self.by_id.insert(ids[i], object.model_matrix);
            }
            self.by_index.push((
                object.mesh_index,
                object.material_index,
                object.model_matrix,
            ));
        }
    }
}

impl super::Renderer {
    /// Works out which objects moved since the last frame and uploads their previous matrices.
    pub(super) fn update_object_motion(&mut self, objects: &[RenderObject]) {
        let ids = self.frame_object_ids.take();
        self.object_motion.update(objects, ids.as_deref());
        let bytes = bytemuck::cast_slice(&self.object_motion.prev_matrices).to_vec();
        self.staging_ring
            .write_immediate(&self.queue, &self.prev_model_buffer, 0, &bytes, false);
    }

    /// Records the pass that overwrites the velocity of moving objects (after the fullscreen
    /// camera-motion pass, which it loads).
    pub(super) fn record_object_velocity(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.object_motion.moving.is_empty() {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("object_velocity_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view: &self.taa_velocity_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.object_velocity_pass.pipeline);
        pass.set_bind_group(0, &self.object_velocity_bg, &[]);
        for &(instance, mesh_slot) in &self.object_motion.moving {
            let Some(mesh) = self.registry.mesh_by_slot(mesh_slot) else {
                continue;
            };
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, instance..instance + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use redox_math::{Aabb, Vec3};

    use super::*;

    fn object(mesh: usize, material: usize, x: f32) -> RenderObject {
        RenderObject {
            model_matrix: Mat4::from_translation(Vec3::new(x, 0.0, 0.0)),
            color: [1.0; 4],
            mesh_index: mesh,
            material_index: material,
            bounds: Aabb::from_center_size(Vec3::ZERO, Vec3::ONE),
        }
    }

    #[test]
    fn the_first_frame_has_no_motion() {
        let mut motion = ObjectMotion::default();
        motion.update(&[object(0, 0, 1.0), object(1, 0, 2.0)], None);
        assert!(motion.moving.is_empty());
        assert_eq!(motion.prev_matrices.len(), 2);
    }

    #[test]
    fn a_moved_object_is_reported_with_its_previous_matrix() {
        let mut motion = ObjectMotion::default();
        motion.update(&[object(3, 0, 0.0), object(4, 0, 5.0)], None);
        motion.update(&[object(3, 0, 0.5), object(4, 0, 5.0)], None);
        assert_eq!(motion.moving, vec![(0, 3)]);
        assert_eq!(motion.prev_matrices[0], Mat4::IDENTITY.to_cols_array_2d());
    }

    #[test]
    fn index_matching_ignores_objects_whose_mesh_or_material_changed() {
        let mut motion = ObjectMotion::default();
        motion.update(&[object(0, 0, 0.0)], None);
        motion.update(&[object(0, 1, 9.0)], None);
        assert!(
            motion.moving.is_empty(),
            "a different object is not a moved one"
        );
    }

    #[test]
    fn ids_follow_objects_when_the_list_changes() {
        let mut motion = ObjectMotion::default();
        motion.update(
            &[object(0, 0, 0.0), object(0, 0, 10.0), object(0, 0, 20.0)],
            Some(&[1, 2, 3]),
        );
        // Object 1 is gone; 2 and 3 shift down by one index and 3 moves.
        motion.update(&[object(0, 0, 10.0), object(0, 0, 21.0)], Some(&[2, 3]));
        assert_eq!(
            motion.moving,
            vec![(1, 0)],
            "only object 3 moved, 2 did not"
        );
        let prev = Mat4::from_cols_array_2d(&motion.prev_matrices[1]);
        assert!(
            (prev.w_axis.x - 20.0).abs() < 1e-6,
            "its own previous position"
        );
    }

    #[test]
    fn ids_of_the_wrong_length_are_ignored() {
        let mut motion = ObjectMotion::default();
        motion.update(&[object(0, 0, 0.0)], Some(&[1, 2, 3]));
        motion.update(&[object(0, 0, 1.0)], Some(&[1, 2, 3]));
        assert_eq!(motion.moving.len(), 1, "falls back to index matching");
    }
}

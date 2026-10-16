//! Shadow preparation: VSM dirty-page selection and the point-light atlas jobs.

use super::super::*;

impl Renderer {
    /// Updates the directional VSM clipmap for this frame and returns the pages that
    /// need (re-)rendering. Empty when VSM is disabled.
    pub(super) fn collect_vsm_dirty_pages(
        &mut self,
        objects: &[RenderObject],
        cam_eye: Vec3,
        light_dir: Vec3,
    ) -> Vec<crate::shadow::vsm_clipmap::DirtyPage> {
        if self.vsm_enabled {
            self.vsm_clipmap.update(cam_eye, light_dir);
            let mut aabbs: Vec<redox_math::Aabb> = objects.iter().map(|o| o.bounds).collect();
            // Always request pages near the camera frustum, not only object bounds.
            let frustum_pad = self.camera_far * 0.15;
            aabbs.push(redox_math::Aabb {
                min: cam_eye - redox_math::Vec3::splat(frustum_pad),
                max: cam_eye + redox_math::Vec3::splat(frustum_pad),
            });
            let dynamic_aabbs: Vec<redox_math::Aabb> = objects
                .iter()
                .filter(|o| {
                    let ext = o.bounds.max - o.bounds.min;
                    !(ext.y < 0.6 && ext.x > 8.0 && ext.z > 8.0) // ground
                        && ext.x < 6.0 && ext.z < 6.0 && ext.y < 4.0 // small dynamic
                })
                .map(|o| o.bounds)
                .collect();
            // Debounce dynamic invalidation: only if centers moved >0.5м (prevents 4м grid thrash)
            let cur_centers: Vec<redox_math::Vec3> = dynamic_aabbs
                .iter()
                .map(|a| (a.min + a.max) * 0.5)
                .collect();
            let need_invalidate = if cur_centers.len() != self.prev_dynamic_centers.len() {
                true
            } else {
                cur_centers
                    .iter()
                    .zip(self.prev_dynamic_centers.iter())
                    .any(|(c, p)| (*c - *p).length() > 0.5)
            };
            if need_invalidate {
                self.prev_dynamic_centers = cur_centers;
                self.vsm_clipmap.invalidate_aabbs(&dynamic_aabbs);
            }
            // Fine pages come from the screen feedback (what the pixels actually see); the
            // two coarsest levels are requested for the scene bounds as a safety net while
            // that feedback is a frame or two behind.
            let mut requested = self.vsm_clipmap.coarse_fallback_bits(&aabbs, 2);
            for (word, fed) in requested
                .iter_mut()
                .zip(self.vsm_page_request.latest_bits())
            {
                *word |= *fed;
            }
            let dirty = self.vsm_clipmap.requested_dirty_pages(&requested);
            self.vsm_dirty_pages_last_frame = dirty.len() as u32;
            self.vsm_warm = dirty.is_empty();
            dirty
        } else {
            Vec::new()
        }
    }

    /// Point-light cube shadows: multi-caster tiled atlas.
    ///
    /// The atlas is 8192² of 512² tiles (16×16 grid = 256 tiles = 42 casters). Casters
    /// are prioritised nearest-to-camera; overflow lights render unshadowed (`NO_TILE`
    /// in the GPU light struct). Also patches the clustered-lights buffer with each
    /// caster's face matrices and tile base.
    pub(super) fn prepare_point_atlas_jobs(
        &mut self,
        objects: &[RenderObject],
        cam_eye: Vec3,
    ) -> Vec<AtlasCasterJob> {
        use crate::shadow::atlas_allocator::{NO_TILE, ShadowAtlasAllocator};

        // Sort casters by distance to the camera (closest first).
        let mut caster_order: Vec<(usize, f32)> = self
            .current_point_lights
            .iter()
            .enumerate()
            .filter(|(_, l)| l.cast_shadows)
            .map(|(i, l)| (i, (l.position - cam_eye).length_squared()))
            .collect();
        caster_order.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut allocator = ShadowAtlasAllocator::new();
        allocator.begin_frame();
        let mut tile_bases: Vec<(usize, u32)> = Vec::new();

        let mut local_shadow_matrices = [Mat4::IDENTITY; 6];
        let face_forward_up: [(Vec3, Vec3); 6] = [
            (Vec3::X, Vec3::Y),     // PosX
            (Vec3::NEG_X, Vec3::Y), // NegX
            (Vec3::Y, Vec3::NEG_Z), // PosY
            (Vec3::NEG_Y, Vec3::Z), // NegY
            (Vec3::Z, Vec3::Y),     // PosZ
            (Vec3::NEG_Z, Vec3::Y), // NegZ
        ];

        let mut atlas_jobs: Vec<AtlasCasterJob> = Vec::new();
        for (caster_ord, &(light_idx, _)) in caster_order.iter().enumerate() {
            let tile_base = allocator.allocate(light_idx as u64);
            if tile_base == NO_TILE {
                continue;
            }
            if caster_ord >= crate::shadow::atlas_allocator::MAX_CASTERS as usize {
                break;
            }
            tile_bases.push((light_idx, tile_base));

            let light = &self.current_point_lights[light_idx];
            let light_pos = light.position;
            let near = 0.05_f32;
            let far = light.radius.max(0.5);
            let proj =
                redox_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
            for (fi, (fwd, up)) in face_forward_up.iter().enumerate() {
                let target = light_pos + *fwd;
                let view = redox_math::Mat4::look_at_rh(light_pos, target, *up);
                local_shadow_matrices[fi] = proj * view;
            }

            let face_base = caster_ord * 6;
            for (face_i, m) in local_shadow_matrices.iter().enumerate() {
                self.queue.write_buffer(
                    &self.face_matrix_buffers[face_base + face_i],
                    0,
                    bytemuck::bytes_of(&m.to_cols_array_2d()),
                );
            }

            let face_vis: [Vec<bool>; 6] = std::array::from_fn(|face_i| {
                let frustum =
                    redox_math::Frustum::from_view_projection(local_shadow_matrices[face_i]);
                objects
                    .iter()
                    .map(|o| frustum.intersects_aabb(&o.bounds))
                    .collect()
            });
            atlas_jobs.push(AtlasCasterJob {
                face_base,
                tile_base,
                face_vis,
            });
        }
        // Patch every caster's GPU entry with its face matrices + tile base,
        // then refresh the clustered lights storage buffer.
        if !tile_bases.is_empty() {
            use crate::light::PointLightGpu;
            let mut gpu_lights: Vec<PointLightGpu> = self
                .current_point_lights
                .iter()
                .map(PointLightGpu::from_point_light)
                .collect();

            for &(light_idx, tile_base) in &tile_bases {
                // Recompute this light's matrices (local_shadow_matrices holds
                // only the LAST caster's values at this point).
                let light = &self.current_point_lights[light_idx];
                let near = 0.05_f32;
                let far = light.radius.max(0.5);
                let proj =
                    redox_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
                for (fi, (fwd, up)) in face_forward_up.iter().enumerate() {
                    let view =
                        redox_math::Mat4::look_at_rh(light.position, light.position + *fwd, *up);
                    gpu_lights[light_idx].shadow_matrices[fi] = (proj * view).to_cols_array_2d();
                }
                gpu_lights[light_idx].shadow_tile_base = tile_base;
            }

            let lights_bytes = bytemuck::cast_slice::<PointLightGpu, u8>(&gpu_lights);
            self.queue
                .write_buffer(&self.cluster_manager.point_lights_buffer, 0, lights_bytes);
        }

        atlas_jobs
    }
}

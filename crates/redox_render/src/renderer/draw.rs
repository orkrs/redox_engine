//! Per-object draw recording (opaque and transparent).

use super::*;

impl Renderer {
    /// Renders a single frame given a list of [`RenderObject`]s.
    ///
    pub fn begin_pass<'a>(&'a self, encoder: &'a mut wgpu::CommandEncoder) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("forward_pass"),
            color_attachments: &[Some(Self::hdr_color_attachment(
                &self.hdr_view,
                &self.hdr_msaa_view,
                self.msaa_sample_count,
                wgpu::LoadOp::Clear(wgpu::Color {
                    r: 0.22,
                    g: 0.26,
                    b: 0.34,
                    a: 1.0,
                }),
                HdrGeometryResolvePass::Forward,
                HdrGeometryResolvePass::Forward,
            ))],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// Records the draw call for one object into `pass`.
    pub fn record_draw<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        debug_assert!(visibility.is_empty() || visibility.len() == objects.len());
        render_pass.set_pipeline(&self.pbr_pass.pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        let mut calls = 0u32;
        let same = |a: usize, b: usize| {
            objects[a].mesh_index == objects[b].mesh_index
                && objects[a].material_index == objects[b].material_index
        };
        for run in batching::runs(
            objects.len(),
            |i| visibility.is_empty() || visibility[i],
            same,
        ) {
            let obj = &objects[run.start as usize];
            // Material bind group (Group 2)
            let material_bg = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .unwrap_or_else(|| {
                    self.registry
                        .legacy_material_bind_group(0)
                        .expect("fallback material")
                });

            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, run);
                calls += 1;
            }
        }
        self.forward_draw_calls
            .store(calls, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns true when any visible object uses an alpha-blended material.
    ///
    /// Cheap pre-check so the transparent render pass is not opened for
    /// fully opaque frames.
    pub fn has_alpha_objects(&self, objects: &[RenderObject], visibility: &[bool]) -> bool {
        objects.iter().enumerate().any(|(i, obj)| {
            (visibility.is_empty() || visibility[i])
                && self
                    .material_blend_flags
                    .get(obj.material_index)
                    .copied()
                    .unwrap_or(BlendMode::Opaque)
                    != BlendMode::Opaque
        })
    }

    /// Draws alpha-blended objects into the HDR target with depth read-only.
    ///
    /// Objects are sorted back-to-front by distance from the camera
    /// (farthest drawn first) so alpha compositing order is correct.
    /// Instance indices stay GLOBAL — they index the model storage buffer,
    /// which holds all objects, not just the transparent ones.
    pub fn record_draw_transparent<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        let cam_pos = Vec3::new(
            self.camera_uniform.camera_pos[0],
            self.camera_uniform.camera_pos[1],
            self.camera_uniform.camera_pos[2],
        );

        // Collect visible alpha-blended object indices with their distance.
        let mut order: Vec<(usize, f32)> = objects
            .iter()
            .enumerate()
            .filter(|(i, obj)| {
                (visibility.is_empty() || visibility[*i])
                    && self
                        .material_blend_flags
                        .get(obj.material_index)
                        .copied()
                        .unwrap_or(BlendMode::Opaque)
                        == BlendMode::Alpha
            })
            .map(|(i, obj)| {
                let c = obj.bounds.center();
                (i, (c - cam_pos).length_squared())
            })
            .collect();

        // Farthest first (descending distance).
        order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if order.is_empty() {
            return;
        }

        render_pass.set_pipeline(&self.transparent_pass.pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        for (i, _) in order {
            let obj = &objects[i];
            let material_bg = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .unwrap_or_else(|| {
                    self.registry
                        .legacy_material_bind_group(0)
                        .expect("fallback material")
                });

            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
            }
        }
    }

    /// Draws additive objects (MAT-06) into the HDR target after the alpha pass.
    ///
    /// Additive blending is commutative, so no sorting is needed. Depth is
    /// tested against the opaque results but not written, as for alpha.
    pub fn record_draw_additive<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        objects: &[RenderObject],
        visibility: &[bool],
    ) {
        let indices: Vec<usize> = objects
            .iter()
            .enumerate()
            .filter(|(i, obj)| {
                (visibility.is_empty() || visibility[*i])
                    && self
                        .material_blend_flags
                        .get(obj.material_index)
                        .copied()
                        .unwrap_or(BlendMode::Opaque)
                        == BlendMode::Additive
            })
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            return;
        }

        render_pass.set_pipeline(&self.transparent_pass.additive_pipeline);
        render_pass.set_bind_group(0, &self.frame_bind_group, &[]);
        render_pass.set_bind_group(1, &self.scene_bind_group, &[]);
        render_pass.set_bind_group(3, &self.model_bind_group, &[]);

        for i in indices {
            let obj = &objects[i];
            let Some(material_bg) = self
                .registry
                .legacy_material_bind_group(obj.material_index)
                .or_else(|| self.registry.legacy_material_bind_group(0))
            else {
                continue;
            };
            render_pass.set_bind_group(2, material_bg, &[]);
            if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..gpu_mesh.index_count, 0, (i as u32)..(i as u32 + 1));
            }
        }
    }
}

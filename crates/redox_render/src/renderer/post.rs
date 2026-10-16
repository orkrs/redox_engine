//! Post-process recording helpers (scatter light, bloom) and VG bind-group rebuilds.

use super::*;

impl Renderer {
    /// ScatterLight fullscreen pass on the HDR buffer (no bloom — bloom runs after TAA).
    pub(super) fn record_scatter_light_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.uses_scatter_light() {
            let uniform = ScatterLightUniform {
                num_lights: self.current_point_lights.len() as u32,
                sample_count: crate::pass::scatter_light::SCATTER_LIGHT_SAMPLE_COUNT,
                frame_index: self.taa_frame_count as u32,
                _pad: 0,
                resolution: [self.internal_width as f32, self.internal_height as f32],
                _pad2: [0.0; 2],
                inv_view_proj: Mat4::from_cols_array_2d(&self.camera_uniform.view_proj)
                    .inverse()
                    .to_cols_array_2d(),
            };
            self.queue.write_buffer(
                &self.scatter_light_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&uniform),
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scatter_light_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.scatter_light_pass.draw(&mut pass, &self.scatter_light_bind_group);
        }
    }

    /// Dual-filter bloom mip chain from a clean HDR source (TAA output or raw HDR).
    pub(super) fn record_bloom_chain(&self, encoder: &mut wgpu::CommandEncoder, source_view: &wgpu::TextureView) {
        if !self.bloom_enabled {
            return;
        }

        use crate::post::bloom::BLOOM_MIP_COUNT;

        let bloom_src_bg = self.bloom_pass.create_bind_group(
            &self.device,
            source_view,
            &self.hdr_sampler,
        );

        for i in 0..BLOOM_MIP_COUNT {
            let pipeline = if i == 0 {
                &self.bloom_pass.downsample_threshold_pipeline
            } else {
                &self.bloom_pass.downsample_pipeline
            };
            let bg = if i == 0 {
                &bloom_src_bg
            } else {
                &self.bloom_down_bgs[i]
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom_downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_mip_views[i],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }

        for i in (0..BLOOM_MIP_COUNT - 1).rev() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom_upsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_mip_views[i],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.bloom_pass.upsample_pipeline);
            pass.set_bind_group(0, &self.bloom_up_bgs[i], &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub(super) fn rebuild_vg_bind_groups(&mut self) {
        let materials: Vec<Material> = self.registry.legacy_materials().to_vec();
        self.vg_material_buffer =
            build_material_storage(&materials, &self.device, &self.queue);

        if let Some(vg) = &self.vg_system {
            self.vg_resolve_bind_group = self.vg_resolve_pass.create_resolve_bind_group(
                &self.device,
                &self.vg_vis_targets.id_view,
                &self.vg_vis_targets.normal_depth_view,
                &self.vg_vis_targets.uv_view,
                &self.depth_view,
                vg.instance_buffer(),
                vg.meshlet_buffer(),
                &self.vg_material_buffer,
            );
            self.vg_shadow_instances_bg = self
                .vg_shadow_pass
                .create_instances_bind_group(&self.device, vg.instance_buffer());
        }
        self.vg_bind_groups_dirty = false;
    }

    // ----- Resource upload helpers -----
}

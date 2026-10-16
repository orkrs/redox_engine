//! Self-contained render passes extracted from `render_frame_into`.

use super::*;

impl Renderer {
    /// Normal prepass (normals + linear depth for GTAO); skipped when pass recording is multi-threaded.
    pub(super) fn record_normal_pass(&mut self, encoder: &mut wgpu::CommandEncoder, objects: &[RenderObject]) {
        if !self.mt_command_recording {
            let mut normal_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("normal_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.normal_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
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
            });
            normal_render_pass.set_pipeline(&self.normal_pass.pipeline);
            normal_render_pass.set_bind_group(0, &self.normal_bind_group, &[]);
            for (i, obj) in objects.iter().enumerate() {
                if !self.main_vis_scratch[i] {
                    continue;
                }
                if let Some(gpu_mesh) = self.registry.mesh_by_slot(obj.mesh_index) {
                    normal_render_pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
                    normal_render_pass.set_index_buffer(
                        gpu_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    normal_render_pass.draw_indexed(
                        0..gpu_mesh.index_count,
                        0,
                        (i as u32)..(i as u32 + 1),
                    );
                }
            }
        }
    }

    /// GTAO raw -> blur -> upsample; clears the AO target to white-ish defaults when disabled.
    pub(super) fn record_gtao_passes(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.ssao_enabled {
            let mut gtao_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_raw_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            gtao_render_pass.set_pipeline(&self.ssao_pass.pipeline);
            gtao_render_pass.set_bind_group(0, &self.ssao_bind_group, &[]);
            gtao_render_pass.draw(0..3, 0..1);
            drop(gtao_render_pass);

            let mut blur_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_blur_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_half_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            blur_render_pass.set_pipeline(&self.ssao_pass.blur_pipeline);
            blur_render_pass.set_bind_group(0, &self.ssao_blur_bind_group, &[]);
            blur_render_pass.draw(0..3, 0..1);
            drop(blur_render_pass);

            let mut upsample_render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_upsample_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            upsample_render_pass.set_pipeline(&self.ssao_pass.upsample_pipeline);
            upsample_render_pass.set_bind_group(0, &self.ssao_upsample_bind_group, &[]);
            upsample_render_pass.draw(0..3, 0..1);
        } else {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gtao_disabled_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.ssao_blurred_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 1.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
    }

    /// VSM screen-space shadow trace + temporal filter (or a clear when disabled).
    pub(super) fn record_vsm_screen_shadow(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.vsm_enabled && self.vsm_screen_shadows {
            self.queue.write_buffer(
                &self.vsm_shadow_pass.pass_uniform_buffer,
                0,
                bytemuck::bytes_of(&VsmPassUniform {
                    frame_index: self.vsm_shadow_frame_count as u32,
                    _pad: [0; 3],
                }),
            );
            self.queue.write_buffer(
                &self.vsm_shadow_pass.temporal_uniform_buffer,
                0,
                bytemuck::bytes_of(&VsmTemporalUniform {
                    blend_alpha: self.vsm_shadow_temporal_alpha,
                    reset: u32::from(self.vsm_shadow_frame_count == 0),
                    _pad: [0.0; 2],
                }),
            );

            let mut trace_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vsm_shadow_trace"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.vsm_shadow_raw_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            trace_pass.set_pipeline(&self.vsm_shadow_pass.trace_pipeline);
            trace_pass.set_bind_group(0, &self.vsm_shadow_trace_bind_group, &[]);
            trace_pass.draw(0..3, 0..1);
            drop(trace_pass);

            let mut temporal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vsm_shadow_temporal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.vsm_shadow_filtered_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            temporal_pass.set_pipeline(&self.vsm_shadow_pass.temporal_pipeline);
            temporal_pass.set_bind_group(0, &self.vsm_shadow_temporal_bind_group, &[]);
            temporal_pass.draw(0..3, 0..1);
            drop(temporal_pass);

            // Ping-pong: output buffer becomes next frame's history (no GPU copy).
            std::mem::swap(&mut self.vsm_shadow_filtered_texture, &mut self.vsm_shadow_history_texture);
            std::mem::swap(&mut self.vsm_shadow_filtered_view, &mut self.vsm_shadow_history_view);
            self.vsm_shadow_temporal_bind_group = self.vsm_shadow_pass.create_temporal_bind_group(
                &self.device,
                &self.vsm_shadow_raw_view,
                &self.vsm_shadow_history_view,
                &self.normal_view,
                &self.camera_buffer,
                &self.common_sampler,
            );
            self.update_global_bind_group();
        } else {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vsm_shadow_disabled_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.vsm_shadow_filtered_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
    }
}

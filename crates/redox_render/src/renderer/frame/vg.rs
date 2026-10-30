//! VG passes: cluster culling and visibility resolve (moved verbatim from frame.rs).

use super::super::*;
use super::FrameState;

/// Screen-space error (pixels) below which a cluster of a DAG asset is precise enough.
const DAG_ERROR_THRESHOLD_PX: f32 = 1.0;

impl Renderer {
    pub(super) fn pass_cluster_vg_cull(&mut self, f: &mut FrameState<'_>) {
        let encoder = &mut f.encoder;
        // ── GPU cluster light cull (optional; CPU fallback in update_clusters) ─
        if self.cluster_manager.gpu_cull {
            self.cluster_manager
                .record_gpu_cull(&self.device, encoder, &self.queue);
        }

        if let Some(ps) = self.particle_system.as_mut()
            && ps.gpu_sim_enabled
            && ps.last_alive_count > 0
        {
            let mut uniforms = self.particle_sim_uniforms;
            uniforms.capacity = ps.capacity;
            uniforms.alive_count = ps.last_alive_count;
            if uniforms.delta_time <= 0.0 {
                uniforms.delta_time = 1.0 / 60.0;
            }
            ps.record_gpu_sim(&self.device, encoder, &self.queue, &uniforms);
        }

        // ── VG GPU cull (optional; CPU cull done in prepare_vg_frame) ───────
        if let Some(vg) = self.vg_system.as_mut() {
            if vg.config.gpu_cull {
                vg.poll_gpu_cull_count(&self.device);
            }
            if vg.config.gpu_cull && vg.active_instance_count() > 0 {
                let cam_pos = Vec3::new(
                    self.camera_uniform.camera_pos[0],
                    self.camera_uniform.camera_pos[1],
                    self.camera_uniform.camera_pos[2],
                );
                vg.record_gpu_cull(
                    &self.device,
                    &self.queue,
                    encoder,
                    &self.camera_uniform.view_proj,
                    cam_pos,
                    &self.hiz,
                );
            } else {
                vg.upload_meshlets_if_dirty(&self.queue);
            }
            // Hierarchical (DAG) assets: select the clusters of this frame on the GPU.
            if vg.dag_instance_count() > 0 {
                let cam_pos = Vec3::new(
                    self.camera_uniform.camera_pos[0],
                    self.camera_uniform.camera_pos[1],
                    self.camera_uniform.camera_pos[2],
                );
                // Pixels per unit at 1 m: half the height in pixels times cot(fov / 2).
                let pixels_per_unit =
                    self.internal_height as f32 * 0.5 * self.camera_uniform.proj[1][1];
                vg.record_dag_traverse(
                    &self.device,
                    &self.queue,
                    encoder,
                    &self.camera_uniform.view_proj,
                    cam_pos,
                    &self.hiz,
                    pixels_per_unit,
                    DAG_ERROR_THRESHOLD_PX,
                );
            }
        }
    }

    pub(super) fn pass_vg_visibility_resolve(&mut self, f: &mut FrameState<'_>) {
        let encoder = &mut f.encoder;
        let hdr_resolve_pass = f.hdr_resolve_pass;
        let msaa_sample_count = self.msaa_sample_count;
        // ── 5a. VG visibility buffer + PBR resolve ───────────────────────────
        if self.vg_use_vis_path {
            let vg_has_content = self
                .vg_system
                .as_ref()
                .map(|v| v.has_content())
                .unwrap_or(false);
            if vg_has_content {
                if self.vg_bind_groups_dirty {
                    self.rebuild_vg_bind_groups();
                }
                if let Some(vg) = &self.vg_system {
                    {
                        let mut vis_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("vg_vis_pass"),
                            color_attachments: &[
                                Some(wgpu::RenderPassColorAttachment {
                                    depth_slice: None,
                                    view: &self.vg_vis_targets.id_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        // R32Uint clear: do NOT use f32::from_bits(0xFFFF_FFFF)
                                        // (that is NaN; many drivers clear uint as 0). wgpu casts
                                        // Color.r as u32, and VIS_ID_SENTINEL as f64 is exact.
                                        load: wgpu::LoadOp::Clear(wgpu::Color {
                                            r: VIS_ID_SENTINEL as f64,
                                            g: 0.0,
                                            b: 0.0,
                                            a: 1.0,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                }),
                                Some(wgpu::RenderPassColorAttachment {
                                    depth_slice: None,
                                    view: &self.vg_vis_targets.normal_depth_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                }),
                                Some(wgpu::RenderPassColorAttachment {
                                    depth_slice: None,
                                    view: &self.vg_vis_targets.uv_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                }),
                            ],
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &self.depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        vg.draw_vis_indirect(
                            &mut vis_pass,
                            &self.vg_vis_pass,
                            &self.frame_bind_group,
                        );
                        vg.draw_dag_vis_indirect(
                            &mut vis_pass,
                            &self.vg_vis_pass,
                            &self.frame_bind_group,
                        );
                    }

                    {
                        let mut resolve_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("vg_resolve_pass"),
                                color_attachments: &[Some(Self::hdr_color_attachment(
                                    &self.hdr_view,
                                    &self.hdr_msaa_view,
                                    msaa_sample_count,
                                    wgpu::LoadOp::Load,
                                    hdr_resolve_pass,
                                    HdrGeometryResolvePass::VgResolve,
                                ))],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                        self.vg_resolve_pass.draw(
                            &mut resolve_pass,
                            &self.frame_bind_group,
                            &self.scene_bind_group,
                            &self.vg_resolve_bind_group,
                        );
                    }
                }
            }
        } else if let Some(vg) = &self.vg_system {
            // Debug forward path (teal normals).
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vg_debug_forward"),
                color_attachments: &[Some(Self::hdr_color_attachment(
                    &self.hdr_view,
                    &self.hdr_msaa_view,
                    msaa_sample_count,
                    wgpu::LoadOp::Load,
                    hdr_resolve_pass,
                    HdrGeometryResolvePass::VgDebug,
                ))],
                depth_stencil_attachment: Some(self.geometry_depth_attachment(wgpu::LoadOp::Load)),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if vg.has_content() {
                vg.render(&mut render_pass);
            }
        }
    }
}

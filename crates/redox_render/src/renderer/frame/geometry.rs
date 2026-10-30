//! Main geometry passes: normal prepass + Hi-Z, forward, transparent and MSAA resolve (moved verbatim from frame.rs).

use super::super::*;
use super::FrameState;

impl Renderer {
    pub(super) fn pass_normal_prepass_hiz(&mut self, f: &mut FrameState<'_>) {
        let objects = f.objects;
        let cam_eye = f.cam_eye;
        let encoder = &mut f.encoder;
        if self.mt_command_recording {
            let prepass_ctx = PrepassContext {
                device: &self.device,
                normal_pass: &self.normal_pass,
                normal_bind_group: &self.normal_bind_group,
                normal_view: &self.normal_view,
                surface_view: &self.material_surface_view,
                depth_view: &self.depth_view,
                registry: &self.registry,
            };
            let vis = self.main_vis_scratch.clone();
            let prepass_cmd = record_normal_prepass(&prepass_ctx, objects, &vis);
            f.mt_cmd_buffers.push(prepass_cmd);
        }
        // 2. Normal pass (normals + linear depth for SSAO)
        self.record_normal_pass(encoder, objects);
        if self.vsm_enabled && !self.probe_capture_active {
            let view_proj = Mat4::from_cols_array_2d(&self.camera_uniform.view_proj);
            self.vsm_page_request.record(
                &self.device,
                &self.queue,
                encoder,
                &self.depth_view,
                &self.vsm_info_buffer,
                view_proj,
                cam_eye,
                self.camera_uniform.proj[1][1],
                (self.internal_width, self.internal_height),
            );
        }

        // Build Hi-Z pyramid from this frame's depth, then late-cull meshlets
        // that Hi-Z of the *previous* frame rejected (1-frame disocclusion).
        self.hiz
            .build_from_depth(&self.device, &self.queue, encoder, &self.depth_view);
        if let Some(vg) = self.vg_system.as_mut()
            && vg.config.gpu_cull
            && vg.active_instance_count() > 0
        {
            let cam_pos = Vec3::new(
                self.camera_uniform.camera_pos[0],
                self.camera_uniform.camera_pos[1],
                self.camera_uniform.camera_pos[2],
            );
            vg.record_gpu_cull_phase2(
                &self.device,
                &self.queue,
                encoder,
                &self.camera_uniform.view_proj,
                cam_pos,
                &self.hiz,
            );
        }
        if let Some(vg) = self.vg_system.as_mut()
            && vg.dag_instance_count() > 0
        {
            let cam_pos = Vec3::new(
                self.camera_uniform.camera_pos[0],
                self.camera_uniform.camera_pos[1],
                self.camera_uniform.camera_pos[2],
            );
            vg.record_dag_phase2(
                &self.queue,
                &self.device,
                encoder,
                &self.camera_uniform.view_proj,
                cam_pos,
                &self.hiz,
            );
        }
    }

    pub(super) fn pass_forward(&mut self, f: &mut FrameState<'_>) {
        let objects = f.objects;
        let encoder = &mut f.encoder;
        let hdr_resolve_pass = f.hdr_resolve_pass;
        let msaa_sample_count = self.msaa_sample_count;
        let debug_lines = std::mem::take(&mut self.current_debug_lines);
        if !debug_lines.is_empty() {
            self.debug_lines_pass
                .upload_lines(&self.queue, &debug_lines);
        }
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("forward_pass_hdr"),
                color_attachments: &[Some(Self::hdr_color_attachment(
                    &self.hdr_view,
                    &self.hdr_msaa_view,
                    msaa_sample_count,
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.08,
                        a: 1.0,
                    }),
                    hdr_resolve_pass,
                    HdrGeometryResolvePass::Forward,
                ))],
                depth_stencil_attachment: Some(
                    self.geometry_depth_attachment(wgpu::LoadOp::Clear(1.0)),
                ),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Skybox background:
            // opaque fragment overwrites it. Skipped while the dummy (black)
            // IBL placeholder is active.
            if let Some(sb) = &self.skybox_pass
                && !self.ibl_resource.is_dummy()
            {
                sb.draw(&mut render_pass);
            }

            self.record_draw(&mut render_pass, objects, &self.main_vis_scratch);
            if !debug_lines.is_empty() {
                self.debug_lines_pass
                    .draw(&self.camera_bind_group, &mut render_pass);
            }
        }
    }

    pub(super) fn pass_transparent(&mut self, f: &mut FrameState<'_>) {
        let objects = f.objects;
        let encoder = &mut f.encoder;
        let hdr_resolve_pass = f.hdr_resolve_pass;
        let msaa_sample_count = self.msaa_sample_count;
        if self.has_alpha_objects(objects, &self.main_vis_scratch) {
            let mut transparent_render_pass =
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("transparent_pass"),
                    color_attachments: &[Some(Self::hdr_color_attachment(
                        &self.hdr_view,
                        &self.hdr_msaa_view,
                        msaa_sample_count,
                        wgpu::LoadOp::Load,
                        hdr_resolve_pass,
                        HdrGeometryResolvePass::Transparent,
                    ))],
                    depth_stencil_attachment: Some(
                        self.geometry_depth_attachment(wgpu::LoadOp::Load),
                    ),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            self.record_draw_transparent(
                &mut transparent_render_pass,
                objects,
                &self.main_vis_scratch,
            );
            self.record_draw_additive(
                &mut transparent_render_pass,
                objects,
                &self.main_vis_scratch,
            );
        }
    }

    pub(super) fn pass_msaa_resolve_particles(&mut self, f: &mut FrameState<'_>) {
        let encoder = &mut f.encoder;
        if self.aa_sharp && self.msaa_sample_count > 1 {
            self.msaa_resolve_pass
                .record(encoder, &self.msaa_resolve_bg, &self.hdr_view);
        }

        // ── 5d. GPU particles (billboards + reactive mask) ─────────────────
        if self.reactive_view.is_some()
            && let Some(reactive) = &self.reactive_view
        {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reactive_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: reactive,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        if let Some(ps) = &self.particle_system
            && (ps.last_alive_count > 0 || ps.gpu_sim_enabled)
        {
            let reactive = self.reactive_view.as_ref();
            let mut particle_pass =
                ps.begin_draw_pass(encoder, &self.hdr_view, &self.depth_view, reactive);
            ps.record_draw(&mut particle_pass, reactive);
        }
    }
}

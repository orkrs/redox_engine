//! Frame orchestration: `render_frame_into` and its entry points.

use super::*;

mod post;
mod shadows;

/// CPU time spent in the last `render_frame_into`, in milliseconds (wall clock, not GPU time).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameCpuTimings {
    /// Visibility, batching and sorting of the object list.
    pub prepare_ms: f32,
    /// Recording every pass into command buffers.
    pub record_ms: f32,
    /// `queue.submit` and everything after it inside the call.
    pub submit_ms: f32,
}

impl FrameCpuTimings {
    /// Sum of the three phases.
    pub fn total_ms(&self) -> f32 {
        self.prepare_ms + self.record_ms + self.submit_ms
    }
}

fn ms_between(from: std::time::Instant, to: std::time::Instant) -> f32 {
    to.duration_since(from).as_secs_f32() * 1000.0
}

/// State shared by the passes of one frame.
struct FrameState<'a> {
    objects: &'a [RenderObject],
    surface_view: &'a wgpu::TextureView,
    ts_encoder: wgpu::CommandEncoder,
    encoder: wgpu::CommandEncoder,
    mt_cmd_buffers: Vec<wgpu::CommandBuffer>,
    unjittered_vp: Mat4,
    jittered_vp: Mat4,
    cam_eye: Vec3,
    vsm_dirty: Vec<crate::shadow::vsm_clipmap::DirtyPage>,
    hdr_resolve_pass: HdrGeometryResolvePass,
}

impl Renderer {
    /// Returns the next available swap-chain texture so callers can render
    /// 3D + UI into the same surface before calling `present()` once.
    pub fn surface_texture(&self) -> Result<wgpu::SurfaceTexture, crate::surface::SurfaceError> {
        crate::surface::acquire_surface_texture(
            self.surface
                .as_ref()
                .ok_or(crate::surface::SurfaceError::Lost)?,
        )
    }
    /// Renders all 3D passes into `surface_view`.
    ///
    /// Does **not** call `present()` — the caller must do that after adding
    /// any additional passes (e.g. UI) to the same surface texture.
    ///
    /// The passes run in the order of [`crate::frame_graph::frame_pass_order`], which is the
    /// compiled frame graph; each pass is followed by a GPU profiler mark carrying its name.
    pub fn render_frame_into(
        &mut self,
        objects: &[RenderObject],
        surface_view: &wgpu::TextureView,
    ) -> Result<(), crate::surface::SurfaceError> {
        let t0 = std::time::Instant::now();
        // Group objects with the same mesh and material so the draw loops can instance them.
        let mut visible = Vec::new();
        compute_main_visibility(&self.camera_uniform.view_proj, objects, &mut visible);
        let grouped: Vec<RenderObject>;
        let objects = match batching::draw_order(objects, &visible) {
            Some(order) => {
                grouped = batching::permute(objects, &order);
                if let Some(ids) = self.frame_object_ids.as_mut()
                    && ids.len() == order.len()
                {
                    *ids = batching::permute(ids, &order);
                }
                &grouped[..]
            }
            None => objects,
        };
        let t1 = std::time::Instant::now();
        let mut frame = self.begin_frame_state(objects, surface_view);
        for &name in crate::frame_graph::frame_pass_order() {
            self.run_frame_pass(name, &mut frame);
            self.gpu_profiler.mark(&mut frame.encoder, name);
        }
        let t2 = self.finish_frame(frame);
        let t3 = std::time::Instant::now();
        self.last_cpu_timings = FrameCpuTimings {
            prepare_ms: ms_between(t0, t1),
            record_ms: ms_between(t1, t2),
            submit_ms: ms_between(t2, t3),
        };
        Ok(())
    }

    /// Runs the named pass of the frame graph.
    fn run_frame_pass(&mut self, name: &str, f: &mut FrameState<'_>) {
        match name {
            "shadow_passes" => self.pass_shadows(f),
            "ddgi_fill" => self.pass_ddgi_fill(f),
            "cluster_vg_cull" => self.pass_cluster_vg_cull(f),
            "normal_prepass_hiz" => self.pass_normal_prepass_hiz(f),
            "gtao" => self.record_gtao_passes(&mut f.encoder),
            "vsm_screen_shadow" => self.record_vsm_screen_shadow(&mut f.encoder),
            "forward" => self.pass_forward(f),
            "vg_visibility_resolve" => self.pass_vg_visibility_resolve(f),
            "transparent" => self.pass_transparent(f),
            "msaa_resolve_particles" => self.pass_msaa_resolve_particles(f),
            "scatter_light" => self.record_scatter_light_pass(&mut f.encoder),
            "taa_tonemap_bloom_smaa" => {
                self.record_post_chain(
                    &mut f.encoder,
                    f.surface_view,
                    f.jittered_vp,
                    f.unjittered_vp,
                );
            }
            "frame_tail" => self.pass_frame_tail(f),
            other => unreachable!("frame graph pass `{other}` has no implementation"),
        }
    }

    /// CPU-side preparation shared by all passes: buffers, TAA jitter, light uniform, visibility.
    fn begin_frame_state<'a>(
        &mut self,
        objects: &'a [RenderObject],
        surface_view: &'a wgpu::TextureView,
    ) -> FrameState<'a> {
        self.ddgi_gpu.update(
            &self.queue,
            1.0 / 60.0,
            -Vec3::from_array([
                self.light_uniform.dir_direction[0],
                self.light_uniform.dir_direction[1],
                self.light_uniform.dir_direction[2],
            ]),
            Vec3::from_array([
                self.light_uniform.dir_color[0],
                self.light_uniform.dir_color[1],
                self.light_uniform.dir_color[2],
            ]),
            self.light_uniform.dir_color[3],
        );

        self.gpu_profiler.begin_frame(&self.device);
        if self.vsm_enabled {
            self.vsm_page_request.begin_frame(&self.device);
        }
        let mut ts_encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gpu_ts_begin"),
            });
        self.gpu_profiler.mark(&mut ts_encoder, "frame_start");

        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });

        // 0. Update Buffers
        self.update_model_buffer(objects);
        self.update_object_motion(objects);

        // VSM-only policy: keep GPU light flag in sync even if callers forgot
        // to set DirectionalLight::vsm.
        if self.vsm_enabled && self.light_uniform.soft_shadows != 2 {
            self.light_uniform.soft_shadows = 2;
            let bytes = bytemuck::bytes_of(&self.light_uniform).to_vec();
            self.staging_ring
                .write_immediate(&self.queue, &self.light_buffer, 0, &bytes, false);
        }

        // ── TAA jitter injection ────────────────────────────────────────────
        // Jitter **projection** P (view-Z column), then VP' = P' * V.
        // Mutating combined VP.z_axis would scale the shift by world Z.
        let camera_view = Mat4::from_cols_array_2d(&self.camera_uniform.view);
        let camera_proj = Mat4::from_cols_array_2d(&self.camera_uniform.proj);
        let unjittered_vp = camera_proj * camera_view;

        let jittered_vp = if self.taa_enabled {
            let jitter = halton_jitter_ndc(
                self.taa_frame_count,
                self.internal_width,
                self.internal_height,
            );
            let jittered_proj = apply_jitter_to_projection(camera_proj, jitter);
            let jittered_vp = jittered_proj * camera_view;

            let mut jittered_uniform = self.camera_uniform;
            jittered_uniform.proj = jittered_proj.to_cols_array_2d();
            jittered_uniform.inv_proj = jittered_proj.inverse().to_cols_array_2d();
            jittered_uniform.view_proj = jittered_vp.to_cols_array_2d();
            self.queue.write_buffer(
                &self.camera_buffer,
                0,
                bytemuck::bytes_of(&jittered_uniform),
            );
            jittered_vp
        } else {
            unjittered_vp
        };

        // 0. Update Light Uniform with Shadow Matrix
        let mut light_u = self.pbr_pass_light_uniform();
        if self.uses_scatter_light() {
            light_u.point_light_path = 1;
        }
        // Keep screen-space VSM flag in sync (PBR branches on this).
        light_u.vsm_screen_shadows = u32::from(self.vsm_screen_shadows);
        let light_dir = redox_math::Vec3::new(
            light_u.dir_direction[0],
            light_u.dir_direction[1],
            light_u.dir_direction[2],
        )
        .normalize();
        // The classic single shadow map is gone; keep the legacy field neutral.
        light_u.shadow_view_proj = Mat4::IDENTITY.to_cols_array_2d();
        self.update_light_buffer(&light_u);

        let cam_eye = Vec3::new(
            self.camera_uniform.camera_pos[0],
            self.camera_uniform.camera_pos[1],
            self.camera_uniform.camera_pos[2],
        );

        // Collect VSM dirty pages for this frame's clipmap update.
        let vsm_dirty = self.collect_vsm_dirty_pages(objects, cam_eye, light_dir);
        compute_main_visibility(
            &self.camera_uniform.view_proj,
            objects,
            &mut self.main_vis_scratch,
        );

        let hdr_resolve_pass = self.hdr_geometry_resolve_pass(objects, &self.main_vis_scratch);
        FrameState {
            objects,
            surface_view,
            ts_encoder,
            encoder,
            mt_cmd_buffers: Vec::new(),
            unjittered_vp,
            jittered_vp,
            cam_eye,
            vsm_dirty,
            hdr_resolve_pass,
        }
    }

    fn pass_ddgi_fill(&mut self, f: &mut FrameState<'_>) {
        let objects = f.objects;
        let encoder = &mut f.encoder;
        self.ddgi_gpu.record_gpu_fill(
            &self.device,
            &self.queue,
            encoder,
            objects,
            -Vec3::from_array([
                self.light_uniform.dir_direction[0],
                self.light_uniform.dir_direction[1],
                self.light_uniform.dir_direction[2],
            ]),
            Vec3::from_array([
                self.light_uniform.dir_color[0],
                self.light_uniform.dir_color[1],
                self.light_uniform.dir_color[2],
            ]),
            self.light_uniform.dir_color[3],
        );
    }

    fn pass_cluster_vg_cull(&mut self, f: &mut FrameState<'_>) {
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
        }
    }

    /// Point-light atlas and virtual shadow map pages; recorded into separate command buffers
    /// that are submitted before the frame encoder.
    fn pass_shadows(&mut self, f: &mut FrameState<'_>) {
        let objects = f.objects;
        let cam_eye = f.cam_eye;
        let atlas_jobs = self.prepare_point_atlas_jobs(objects, cam_eye);

        let atlas_ctx = AtlasContext {
            device: &self.device,
            shadow_pass: &self.shadow_pass,
            shadow_model_bind_group: &self.shadow_model_bind_group,
            face_matrix_bgs: &self.face_matrix_bgs,
            atlas_view: &self.local_shadow_atlas_view,
            vg_shadow_pass: &self.vg_shadow_pass,
            vg_shadow_instances_bg: &self.vg_shadow_instances_bg,
            registry: &self.registry,
        };

        // ── VSM production (directional; only when the mode is enabled) ──
        // Static-scene-optimal slice: pages render once and stay cached until
        // invalidation (light moved). Dynamic-object page tracking via
        // visibility-bits readback is a documented follow-up.
        if self.vsm_enabled {
            let dirty = std::mem::take(&mut f.vsm_dirty);

            // Upload page tables AFTER allocation so PBR sees VALID bits
            // this frame (uploading first made moving-sun frames fall back
            // to a classic map, then static frames sampled empty cached pages).
            let info = self.vsm_clipmap.gpu_info(&self.smrt_config);
            self.queue
                .write_buffer(&self.vsm_info_buffer, 0, bytemuck::bytes_of(&info));

            // Per-page PageDrawInfo packed into the dynamic-offset uniform
            // (256-byte stride, one entry per dirty page).
            let mut pageinfo_scratch: Vec<u8> = Vec::with_capacity(dirty.len() * 256);
            let mut page_vps: Vec<Mat4> = Vec::with_capacity(dirty.len());
            for &(level, px, py, _phys) in &dirty {
                let vp = self.vsm_clipmap.page_view_proj(level as usize, px, py);
                page_vps.push(vp);
                let mut page_info = [0u8; 80];
                page_info[..64].copy_from_slice(bytemuck::bytes_of(&vp.to_cols_array_2d()));
                pageinfo_scratch.extend_from_slice(&page_info);
                pageinfo_scratch.extend_from_slice(&[0u8; 176]);
            }
            if !pageinfo_scratch.is_empty() {
                self.queue
                    .write_buffer(&self.vsm_pageinfo_buffer, 0, &pageinfo_scratch);
            }

            let vsm_ctx = VsmContext {
                device: &self.device,
                vsm_depth_pass: &self.vsm_depth_pass,
                vsm_pageinfo_bg: &self.vsm_pageinfo_bg,
                vsm_pages_views: &self.vsm_pages_views,
                registry: &self.registry,
            };
            if self.mt_command_recording {
                let (atlas_cmd, vsm_cmd) = join(
                    || {
                        record_point_atlas_shadows(
                            &atlas_ctx,
                            &atlas_jobs,
                            objects,
                            self.vg_system.as_ref(),
                        )
                    },
                    || record_vsm_pages(&vsm_ctx, &dirty, &page_vps, objects),
                );
                f.mt_cmd_buffers.push(atlas_cmd);
                f.mt_cmd_buffers.push(vsm_cmd);
            } else {
                f.mt_cmd_buffers.push(record_point_atlas_shadows(
                    &atlas_ctx,
                    &atlas_jobs,
                    objects,
                    self.vg_system.as_ref(),
                ));
                f.mt_cmd_buffers
                    .push(record_vsm_pages(&vsm_ctx, &dirty, &page_vps, objects));
            }
            self.vsm_clipmap.mark_pages_cached(&dirty);
        } else {
            f.mt_cmd_buffers.push(record_point_atlas_shadows(
                &atlas_ctx,
                &atlas_jobs,
                objects,
                self.vg_system.as_ref(),
            ));
        }
    }

    fn pass_normal_prepass_hiz(&mut self, f: &mut FrameState<'_>) {
        let objects = f.objects;
        let cam_eye = f.cam_eye;
        let encoder = &mut f.encoder;
        if self.mt_command_recording {
            let prepass_ctx = PrepassContext {
                device: &self.device,
                normal_pass: &self.normal_pass,
                normal_bind_group: &self.normal_bind_group,
                normal_view: &self.normal_view,
                depth_view: &self.depth_view,
                registry: &self.registry,
            };
            let vis = self.main_vis_scratch.clone();
            let prepass_cmd = record_normal_prepass(&prepass_ctx, objects, &vis);
            f.mt_cmd_buffers.push(prepass_cmd);
        }
        // 2. Normal pass (normals + linear depth for SSAO)
        self.record_normal_pass(encoder, objects);
        if self.vsm_enabled {
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
    }

    fn pass_forward(&mut self, f: &mut FrameState<'_>) {
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

    fn pass_vg_visibility_resolve(&mut self, f: &mut FrameState<'_>) {
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

    fn pass_transparent(&mut self, f: &mut FrameState<'_>) {
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
        }
    }

    fn pass_msaa_resolve_particles(&mut self, f: &mut FrameState<'_>) {
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

    fn pass_frame_tail(&mut self, f: &mut FrameState<'_>) {
        let encoder = &mut f.encoder;
        // Copy current depth → prev depth for next frame's TAA disocclusion test.
        if self.taa_enabled {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.prev_depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::Extent3d {
                    width: self.internal_width,
                    height: self.internal_height,
                    depth_or_array_layers: 1,
                },
            );
        }

        if let Some(vg) = self.vg_system.as_mut()
            && vg.config.gpu_cull
        {
            vg.schedule_gpu_cull_readback(encoder);
        }

        // Feed next frame's motion reprojection (VSM temporal, future TAA-adjacent passes).
        self.camera_uniform.snapshot_prev_view_proj();
        if self.vsm_enabled {
            self.vsm_shadow_frame_count = self.vsm_shadow_frame_count.saturating_add(1);
        }
    }

    /// Resolves the profiler, submits everything and recycles the staging ring.
    /// Returns the instant just before `queue.submit`, for the CPU timing split.
    fn finish_frame(&mut self, f: FrameState<'_>) -> std::time::Instant {
        let FrameState {
            ts_encoder,
            mut encoder,
            mt_cmd_buffers,
            ..
        } = f;
        self.gpu_profiler.resolve(&mut encoder);
        let submit_start = std::time::Instant::now();
        self.queue.submit(
            std::iter::once(ts_encoder.finish())
                .chain(mt_cmd_buffers)
                .chain(std::iter::once(encoder.finish())),
        );
        self.gpu_profiler.after_submit();
        if self.vsm_enabled {
            // The first frames wait for the feedback so shadows are fine-grained from the start.
            self.vsm_page_request
                .after_submit(&self.device, self.vsm_request_frames < 4);
            self.vsm_request_frames = self.vsm_request_frames.saturating_add(1);
        }
        self.staging_ring.end_frame();
        submit_start
    }

    /// Convenience wrapper: acquires a surface texture, renders into it, then
    /// presents it. Use `render_frame_into` + manual `present()` when you need
    /// to add additional passes (e.g. egui) before presentation.
    /// Render one frame from an explicit [`FrameView`] (preferred API).
    pub fn render_frame_view(
        &mut self,
        fv: &FrameView<'_>,
        surface_view: &wgpu::TextureView,
    ) -> Result<(), crate::surface::SurfaceError> {
        self.camera_near = fv.camera.near;
        self.camera_far = fv.camera.far;
        self.frame_object_ids = fv.object_ids.map(<[u64]>::to_vec);
        let mut light_u = fv.lights.to_legacy_uniform();
        light_u.ambient[3] = self.ibl_intensity;
        self.update_light_buffer(&light_u);
        self.current_point_lights = fv.lights.points.clone();
        self.update_cluster_lights(&fv.lights.points, fv.camera);
        self.render_frame_into(fv.objects, surface_view)
    }

    /// Renders a frame of `objects` into the window surface.
    pub fn render_frame(
        &mut self,
        objects: &[RenderObject],
    ) -> Result<(), crate::surface::SurfaceError> {
        let output = crate::surface::acquire_surface_texture(
            self.surface
                .as_ref()
                .ok_or(crate::surface::SurfaceError::Lost)?,
        )?;
        let surface_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_frame_into(objects, &surface_view)?;
        self.queue.present(output);
        Ok(())
    }
}

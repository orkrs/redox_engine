//! Frame orchestration: `render_frame_into` and its entry points.

use super::*;

impl Renderer {
    /// Returns the next available swap-chain texture so callers can render
    /// 3D + UI into the same surface before calling `present()` once.
    pub fn surface_texture(&self) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError> {
        self.surface
            .as_ref()
            .ok_or(wgpu::SurfaceError::Lost)?
            .get_current_texture()
    }

    /// Renders all 3D passes into `surface_view`.
    ///
    /// Does **not** call `present()` — the caller must do that after adding
    /// any additional passes (e.g. UI) to the same surface texture.
    pub fn render_frame_into(
        &mut self,
        objects: &[RenderObject],
        surface_view: &wgpu::TextureView,
    ) -> Result<(), wgpu::SurfaceError> {
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

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_encoder"),
            });
        // Everything submitted before this encoder (shadow page / atlas passes) ends here.
        self.gpu_profiler.mark(&mut encoder, "shadow_passes");

        self.ddgi_gpu.record_gpu_fill(
            &self.device,
            &self.queue,
            &mut encoder,
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

        self.gpu_profiler.mark(&mut encoder, "ddgi_fill");

        // 0. Update Buffers
        self.update_model_buffer(objects);

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

        // ── GPU cluster light cull (optional; CPU fallback in update_clusters) ─
        if self.cluster_manager.gpu_cull {
            self.cluster_manager
                .record_gpu_cull(&self.device, &mut encoder, &self.queue);
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
            ps.record_gpu_sim(&self.device, &mut encoder, &self.queue, &uniforms);
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
                    &mut encoder,
                    &self.camera_uniform.view_proj,
                    cam_pos,
                    &self.hiz,
                );
            } else {
                vg.upload_meshlets_if_dirty(&self.queue);
            }
        }

        self.gpu_profiler.mark(&mut encoder, "cluster_vg_cull");
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
        let vsm_dirty: Vec<crate::shadow::vsm_clipmap::DirtyPage> = if self.vsm_enabled {
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
        };
        compute_main_visibility(
            &self.camera_uniform.view_proj,
            objects,
            &mut self.main_vis_scratch,
        );

        let mut mt_cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
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
            mt_cmd_buffers.push(prepass_cmd);
        }

        // --- 1b. Point-light cube shadows: MULTI-CASTER, tiled atlas ---
        // Atlas is 8192² of 512² tiles (16×16 grid = 256 tiles = 42 casters).
        // Casters are prioritised nearest-to-camera; overflow lights render
        // unshadowed (`NO_TILE` in the GPU light struct).

        use crate::shadow::atlas_allocator::{
            ATLAS_PX, NO_TILE, ShadowAtlasAllocator, TILE_PX, TILES_PER_AXIS,
        };

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
        let _ = (ATLAS_PX, TILES_PER_AXIS, TILE_PX);

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
            let dirty = vsm_dirty;

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
                mt_cmd_buffers.push(atlas_cmd);
                mt_cmd_buffers.push(vsm_cmd);
            } else {
                mt_cmd_buffers.push(record_point_atlas_shadows(
                    &atlas_ctx,
                    &atlas_jobs,
                    objects,
                    self.vg_system.as_ref(),
                ));
                mt_cmd_buffers.push(record_vsm_pages(&vsm_ctx, &dirty, &page_vps, objects));
            }
            self.vsm_clipmap.mark_pages_cached(&dirty);
        } else {
            mt_cmd_buffers.push(record_point_atlas_shadows(
                &atlas_ctx,
                &atlas_jobs,
                objects,
                self.vg_system.as_ref(),
            ));
        }

        // 2. Normal pass (normals + linear depth for SSAO)
        self.record_normal_pass(&mut encoder, objects);
        if self.vsm_enabled {
            let view_proj = Mat4::from_cols_array_2d(&self.camera_uniform.view_proj);
            self.vsm_page_request.record(
                &self.device,
                &self.queue,
                &mut encoder,
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
            .build_from_depth(&self.device, &self.queue, &mut encoder, &self.depth_view);
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
                &mut encoder,
                &self.camera_uniform.view_proj,
                cam_pos,
                &self.hiz,
            );
        }

        self.gpu_profiler.mark(&mut encoder, "normal_prepass_hiz");
        // 3–5. GTAO (optional — when disabled leave bent normal + AO at defaults)
        self.record_gtao_passes(&mut encoder);

        self.gpu_profiler.mark(&mut encoder, "gtao");
        // 5b. VSM screen-space shadow (1 Vogel ray + temporal) — now default ON.
        // PBR samples filtered R16Float instead of inline 16-ray SMRT.
        // 5b. VSM screen-space shadow (1 Vogel ray + temporal) — now default ON.
        // PBR samples filtered R16Float instead of inline 16-ray SMRT.
        self.record_vsm_screen_shadow(&mut encoder);

        self.gpu_profiler.mark(&mut encoder, "vsm_screen_shadow");
        // 5. Forward pass into HDR. Always clear + draw regular meshes.
        // VG vis/resolve composites on top; skipping this left HDR uninitialized
        // (full-screen white) as soon as any VG instance existed.
        let hdr_resolve_pass = self.hdr_geometry_resolve_pass(objects, &self.main_vis_scratch);
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

        self.gpu_profiler.mark(&mut encoder, "forward");
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
                                    view: &self.vg_vis_targets.normal_depth_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                }),
                                Some(wgpu::RenderPassColorAttachment {
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
            });
            if vg.has_content() {
                vg.render(&mut render_pass);
            }
        }
        self.gpu_profiler
            .mark(&mut encoder, "vg_visibility_resolve");
        // ── 5b. Transparent pass:
        // Same HDR target and depth buffer as the forward pass, but with
        // depth writes disabled (read-only depth) so transparent surfaces
        // never occlude each other.
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
                });
            self.record_draw_transparent(
                &mut transparent_render_pass,
                objects,
                &self.main_vis_scratch,
            );
        }

        self.gpu_profiler.mark(&mut encoder, "transparent");
        // ── 5c. Smart Resolve (Sharp AA): luma-weighted MSAA resolve → hdr_view ─
        if self.aa_sharp && self.msaa_sample_count > 1 {
            self.msaa_resolve_pass
                .record(&mut encoder, &self.msaa_resolve_bg, &self.hdr_view);
        }

        // ── 5d. GPU particles (billboards + reactive mask) ─────────────────
        if self.reactive_view.is_some()
            && let Some(reactive) = &self.reactive_view
        {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reactive_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
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
            });
        }
        if let Some(ps) = &self.particle_system
            && (ps.last_alive_count > 0 || ps.gpu_sim_enabled)
        {
            let reactive = self.reactive_view.as_ref();
            let mut particle_pass =
                ps.begin_draw_pass(&mut encoder, &self.hdr_view, &self.depth_view, reactive);
            ps.record_draw(&mut particle_pass, reactive);
        }

        self.gpu_profiler
            .mark(&mut encoder, "msaa_resolve_particles");
        // ── ScatterLight (HDR, pre-TAA) ─────────────────────────────────────
        self.record_scatter_light_pass(&mut encoder);

        self.gpu_profiler.mark(&mut encoder, "scatter_light");
        // ── Tone-mapping uniform (shared by both call sites below) ─────────
        self.queue.write_buffer(
            &self.tonemap_uniform_buffer,
            0,
            bytemuck::bytes_of(&TonemapUniform {
                exposure: self.exposure,
                sharpen_enabled: if self.taa_enabled { 1 } else { 0 },
                bloom_enabled: if self.bloom_enabled { 1 } else { 0 },
                rcas_sharpness: if self.taa_enabled {
                    self.rcas_sharpness
                } else {
                    0.0
                },
                output_size: [self.config.width as f32, self.config.height as f32],
            }),
        );

        // ── TAA: velocity + accumulation passes ─────────────────────────────
        if self.taa_enabled {
            // --- 6a. Update velocity uniform (same P'*V as rasterization) ---
            let inv_jittered_vp = jittered_vp.inverse();

            let jitter = halton_jitter_ndc(
                self.taa_frame_count,
                self.internal_width,
                self.internal_height,
            );
            let vel_uniform = VelocityUniform {
                inv_curr_vp: inv_jittered_vp.to_cols_array_2d(),
                prev_vp: self.taa_prev_vp_unjittered.to_cols_array_2d(),
                screen_size: [self.internal_width as f32, self.internal_height as f32],
                jitter_uv: [jitter[0] * 0.5, -jitter[1] * 0.5],
            };
            self.queue.write_buffer(
                &self.velocity_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&vel_uniform),
            );

            // --- 6b. Velocity pass ---
            {
                let mut vel_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("velocity_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.taa_velocity_view,
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
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                vel_pass.set_pipeline(&self.velocity_pass.pipeline);
                vel_pass.set_bind_group(0, &self.velocity_bg, &[]);
                vel_pass.draw(0..3, 0..1);
            }

            // --- 6c. Update TAA uniform ---
            let is_first_frame = self.taa_frame_count == 0;
            let jitter = halton_jitter_ndc(
                self.taa_frame_count,
                self.internal_width,
                self.internal_height,
            );
            let taa_uniform = TaaUniform {
                output_size: [self.config.width as f32, self.config.height as f32],
                input_size: [self.internal_width as f32, self.internal_height as f32],
                blend_alpha: self.taa_blend_alpha,
                reset: if is_first_frame { 1 } else { 0 },
                jitter_uv: [jitter[0] * 0.5, -jitter[1] * 0.5],
            };
            self.queue.write_buffer(
                &self.taa_pass.uniform_buffer,
                0,
                bytemuck::bytes_of(&taa_uniform),
            );

            // Ping-pong: read from history_read_idx, write to the other.
            let history_read_idx = self.taa_history_read_idx;
            let history_write_idx = 1 - history_read_idx;
            // Bind groups are prebuilt per history slot and rebuilt on resize.
            let taa_bg = &self.taa_history_bgs[history_read_idx];

            // --- 6d. TAA accumulation pass → write history ---
            {
                let mut taa_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("taa_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.taa_history_view[history_write_idx],
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
                taa_pass.set_pipeline(&self.taa_pass.pipeline);
                taa_pass.set_bind_group(0, taa_bg, &[]);
                taa_pass.draw(0..3, 0..1);
            }

            // --- 6e. Bloom from stabilized TAA output, then tone map ---
            let taa_out_view = &self.taa_history_view[history_write_idx];
            self.record_bloom_chain(&mut encoder, taa_out_view);

            // Prebuilt bind group over the freshly written history slot.
            let taa_tone_bg = &self.taa_tone_bgs[history_write_idx];
            {
                let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tone_mapping_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: surface_view,
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
                post_pass.set_pipeline(&self.tone_mapping_pipeline);
                post_pass.set_bind_group(0, taa_tone_bg, &[]);
                post_pass.draw(0..3, 0..1);
            }

            // --- 6f. Advance ping-pong and frame counter ---
            self.taa_history_read_idx = history_write_idx;
            self.taa_prev_vp_unjittered = unjittered_vp;
            self.taa_frame_count += 1;
        } else if self.aa_sharp {
            // ── Sharp AA path: ACES→LDR → SMAA → RCAS→surface ────────────
            self.record_bloom_chain(&mut encoder, &self.hdr_view);

            // 1) ACES tonemap HDR+bloom → ldr_view (Rgba8Unorm, no RCAS)
            self.queue.write_buffer(
                &self.tonemap_uniform_buffer,
                0,
                bytemuck::bytes_of(&TonemapUniform {
                    exposure: self.exposure,
                    sharpen_enabled: 0,
                    bloom_enabled: if self.bloom_enabled { 1 } else { 0 },
                    rcas_sharpness: 0.0,
                    output_size: [self.config.width as f32, self.config.height as f32],
                }),
            );
            {
                let mut ldr_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sharp_tonemap_ldr"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.ldr_view,
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
                ldr_pass.set_pipeline(&self.tone_mapping_ldr_pipeline);
                ldr_pass.set_bind_group(0, &self.tone_mapping_ldr_bg, &[]);
                ldr_pass.draw(0..3, 0..1);
            }

            // 2) SMAA 1× on LDR
            self.smaa_pass.record(
                &self.device,
                &self.queue,
                &mut encoder,
                &self.ldr_view,
                &self.smaa_out_view,
            );

            // 3) RCAS sharpen → surface (fs_present_rcas: no ACES, just RCAS)
            self.queue.write_buffer(
                &self.tonemap_rcas_uniform_buffer,
                0,
                bytemuck::bytes_of(&TonemapUniform {
                    exposure: 1.0,
                    sharpen_enabled: 1,
                    bloom_enabled: 0,
                    rcas_sharpness: self.rcas_sharpness,
                    output_size: [self.config.width as f32, self.config.height as f32],
                }),
            );
            {
                let mut rcas_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sharp_rcas_present"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: surface_view,
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
                rcas_pass.set_pipeline(&self.rcas_present_pipeline);
                rcas_pass.set_bind_group(0, &self.tone_mapping_rcas_bg, &[]);
                rcas_pass.draw(0..3, 0..1);
            }
        } else {
            // ── Standard path (no TAA, no Sharp): bloom then tone map from HDR
            self.record_bloom_chain(&mut encoder, &self.hdr_view);
            let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone_mapping_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: surface_view,
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
            post_pass.set_pipeline(&self.tone_mapping_pipeline);
            post_pass.set_bind_group(0, &self.tone_mapping_bind_group, &[]);
            post_pass.draw(0..3, 0..1);
        }

        self.gpu_profiler
            .mark(&mut encoder, "taa_tonemap_bloom_smaa");
        // Copy current depth → prev depth for next frame's TAA disocclusion test.
        if self.taa_enabled {
            encoder.copy_texture_to_texture(
                wgpu::ImageCopyTexture {
                    texture: &self.depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::ImageCopyTexture {
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
            vg.schedule_gpu_cull_readback(&mut encoder);
        }

        // Feed next frame's motion reprojection (VSM temporal, future TAA-adjacent passes).
        self.camera_uniform.snapshot_prev_view_proj();
        if self.vsm_enabled {
            self.vsm_shadow_frame_count = self.vsm_shadow_frame_count.saturating_add(1);
        }

        self.gpu_profiler.mark(&mut encoder, "frame_tail");
        self.gpu_profiler.resolve(&mut encoder);
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
        Ok(())
    }

    /// Convenience wrapper: acquires a surface texture, renders into it, then
    /// presents it. Use `render_frame_into` + manual `present()` when you need
    /// to add additional passes (e.g. egui) before presentation.
    /// Render one frame from an explicit [`FrameView`] (preferred API).
    pub fn render_frame_view(
        &mut self,
        fv: &FrameView<'_>,
        surface_view: &wgpu::TextureView,
    ) -> Result<(), wgpu::SurfaceError> {
        self.camera_near = fv.camera.near;
        self.camera_far = fv.camera.far;
        let mut light_u = fv.lights.to_legacy_uniform();
        light_u.ambient[3] = self.ibl_intensity;
        self.update_light_buffer(&light_u);
        self.current_point_lights = fv.lights.points.clone();
        self.update_cluster_lights(&fv.lights.points, fv.camera);
        self.render_frame_into(fv.objects, surface_view)
    }

    pub fn render_frame(&mut self, objects: &[RenderObject]) -> Result<(), wgpu::SurfaceError> {
        let output = self
            .surface
            .as_ref()
            .ok_or(wgpu::SurfaceError::Lost)?
            .get_current_texture()?;
        let surface_view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_frame_into(objects, &surface_view)?;
        output.present();
        Ok(())
    }
}

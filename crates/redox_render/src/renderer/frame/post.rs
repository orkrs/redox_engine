//! Post-processing chain: bloom, TAA (velocity + accumulation), SMAA/RCAS and tone mapping.

use super::super::*;

impl Renderer {
    /// Records everything between the HDR scene and the swap-chain image.
    pub(super) fn record_post_chain(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        jittered_vp: Mat4,
        unjittered_vp: Mat4,
    ) {
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
                        depth_slice: None,
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
                    multiview_mask: None,
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
                        depth_slice: None,
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
                    multiview_mask: None,
                });
                taa_pass.set_pipeline(&self.taa_pass.pipeline);
                taa_pass.set_bind_group(0, taa_bg, &[]);
                taa_pass.draw(0..3, 0..1);
            }

            // --- 6e. Bloom from stabilized TAA output, then tone map ---
            let taa_out_view = &self.taa_history_view[history_write_idx];
            self.record_bloom_chain(encoder, taa_out_view);

            // Prebuilt bind group over the freshly written history slot.
            let taa_tone_bg = &self.taa_tone_bgs[history_write_idx];
            {
                let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("tone_mapping_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        depth_slice: None,
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
                    multiview_mask: None,
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
            self.record_bloom_chain(encoder, &self.hdr_view);

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
                        depth_slice: None,
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
                    multiview_mask: None,
                });
                ldr_pass.set_pipeline(&self.tone_mapping_ldr_pipeline);
                ldr_pass.set_bind_group(0, &self.tone_mapping_ldr_bg, &[]);
                ldr_pass.draw(0..3, 0..1);
            }

            // 2) SMAA 1× on LDR
            self.smaa_pass.record(
                &self.device,
                &self.queue,
                encoder,
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
                        depth_slice: None,
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
                    multiview_mask: None,
                });
                rcas_pass.set_pipeline(&self.rcas_present_pipeline);
                rcas_pass.set_bind_group(0, &self.tone_mapping_rcas_bg, &[]);
                rcas_pass.draw(0..3, 0..1);
            }
        } else {
            // ── Standard path (no TAA, no Sharp): bloom then tone map from HDR
            self.record_bloom_chain(encoder, &self.hdr_view);
            let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone_mapping_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
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
                multiview_mask: None,
            });
            post_pass.set_pipeline(&self.tone_mapping_pipeline);
            post_pass.set_bind_group(0, &self.tone_mapping_bind_group, &[]);
            post_pass.draw(0..3, 0..1);
        }
    }
}

//! Post-processing chain: bloom, TAA (velocity + accumulation), SMAA/RCAS and tone mapping.

use super::super::*;
use crate::post::post_fx::ChainInput;
use crate::post::tone_mapping::create_tonemap_bind_group;

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
                    crate::post::tsr::rcas_stops(self.render_scale, self.rcas_sharpness)
                } else {
                    0.0
                },
                output_size: [self.config.width as f32, self.config.height as f32],
            }),
        );

        if self.taa_enabled {
            self.record_post_with_taa(encoder, surface_view, jittered_vp, unjittered_vp);
        } else if self.aa_sharp {
            self.record_sharp_aa_post(encoder, surface_view, jittered_vp, unjittered_vp);
        } else {
            self.record_standard_post(encoder, surface_view, jittered_vp, unjittered_vp);
        }
    }

    fn chain_frame(&self) -> ChainFrame<'_> {
        ChainFrame {
            camera: &self.camera_uniform,
            near_far: [self.camera_near, self.camera_far],
            screen: [self.config.width as f32, self.config.height as f32],
            sun_direction: self.light_uniform.dir_direction,
            sun_color: self.light_uniform.dir_color,
        }
    }

    fn post_fx_settings_for_frame(&mut self, encoder: &mut wgpu::CommandEncoder) -> PostFxSettings {
        let mut fx_settings = self.post_fx_settings;
        if let Some(reflect) = fx_settings.gi_reflect
            && self.gi_probes.enabled()
        {
            self.gi_reflect.record(
                &self.device,
                &self.queue,
                encoder,
                &self.camera_uniform,
                &reflect,
                &crate::gi::reflect::ReflectInputs {
                    normal: &self.normal_view,
                    surface: &self.material_surface_view,
                    sdf: &self.gi.global_sdf,
                    grid: self.gi_probes.grid_buffer(),
                    irradiance: self.gi_probes.irradiance_view(),
                    probe_offset: self.gi_probes.probe_offset(),
                },
            );
        } else {
            fx_settings.gi_reflect = None;
        }
        fx_settings
    }

  /// Velocity buffer for motion blur when TAA is off (camera plus moving objects).
    fn record_velocity_for_motion_blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        jittered_vp: Mat4,
    ) {
        if self.post_fx_settings.motion_blur.is_none() {
            return;
        }
        let inv_jittered_vp = jittered_vp.inverse();
        let vel_uniform = VelocityUniform {
            inv_curr_vp: inv_jittered_vp.to_cols_array_2d(),
            prev_vp: self.taa_prev_vp_unjittered.to_cols_array_2d(),
            screen_size: [self.internal_width as f32, self.internal_height as f32],
            jitter_uv: [0.0, 0.0],
        };
        self.queue.write_buffer(
            &self.velocity_pass.uniform_buffer,
            0,
            bytemuck::bytes_of(&vel_uniform),
        );
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
        self.record_object_velocity(encoder);
    }

    fn record_post_with_taa(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        jittered_vp: Mat4,
        unjittered_vp: Mat4,
    ) {
        let inv_jittered_vp = jittered_vp.inverse();

        let jitter = crate::post::tsr::jitter_ndc(
            self.taa_frame_count,
            self.render_scale,
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
        self.record_object_velocity(encoder);

        let is_first_frame = self.taa_frame_count == 0;
        let jitter = crate::post::tsr::jitter_ndc(
            self.taa_frame_count,
            self.render_scale,
            self.internal_width,
            self.internal_height,
        );
        let taa_uniform = TaaUniform {
            output_size: [self.config.width as f32, self.config.height as f32],
            input_size: [self.internal_width as f32, self.internal_height as f32],
            blend_alpha: self.taa_blend_alpha,
            reset: if is_first_frame { 1 } else { 0 },
            jitter_uv: [jitter[0] * 0.5, -jitter[1] * 0.5],
            inv_scale: crate::post::tsr::inv_scale(self.render_scale),
            pad: 0.0,
        };
        self.queue.write_buffer(
            &self.taa_pass.uniform_buffer,
            0,
            bytemuck::bytes_of(&taa_uniform),
        );

        let history_read_idx = self.taa_history_read_idx;
        let history_write_idx = 1 - history_read_idx;
        let taa_bg = &self.taa_history_bgs[history_read_idx];

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

        // Optional post effects on TAA output. History stays sharp: effects write their own targets.
        let fx_settings = self.post_fx_settings_for_frame(encoder);
        let fx_out = self.post_fx.run_chain(
            encoder,
            &self.queue,
            ChainInput::History(history_write_idx),
            &self.chain_frame(),
            &fx_settings,
        );
        let taa_out_view = &self.taa_history_view[history_write_idx];
        let bloom_src = match fx_out {
            Some(i) => &self.post_fx.views[i],
            None => taa_out_view,
        };
        self.record_bloom_chain(encoder, bloom_src);

        let taa_tone_bg = match fx_out {
            Some(i) => &self.post_fx.tone_bgs[i],
            None => &self.taa_tone_bgs[history_write_idx],
        };
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

        self.taa_history_read_idx = history_write_idx;
        self.taa_prev_vp_unjittered = unjittered_vp;
        self.taa_frame_count += 1;
    }

    fn hdr_after_post_fx(
        &self,
        fx_out: Option<usize>,
    ) -> &wgpu::TextureView {
        match fx_out {
            Some(i) => &self.post_fx.views[i],
            None => &self.hdr_view,
        }
    }

    fn record_sharp_aa_post(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        jittered_vp: Mat4,
        unjittered_vp: Mat4,
    ) {
        self.record_velocity_for_motion_blur(encoder, jittered_vp);
        let fx_settings = self.post_fx_settings_for_frame(encoder);
        let fx_out = self.post_fx.run_chain(
            encoder,
            &self.queue,
            ChainInput::Hdr,
            &self.chain_frame(),
            &fx_settings,
        );
        let hdr_src = self.hdr_after_post_fx(fx_out);
        self.record_bloom_chain(encoder, hdr_src);

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

        let ldr_bg = match fx_out {
            Some(i) => create_tonemap_bind_group(
                &self.device,
                &self.tone_mapping_ldr_pipeline,
                &self.post_fx.views[i],
                &self.bloom_mip_views[0],
                &self.hdr_sampler,
                &self.tonemap_uniform_buffer,
            ),
            None => self.tone_mapping_ldr_bg.clone(),
        };
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
            ldr_pass.set_bind_group(0, &ldr_bg, &[]);
            ldr_pass.draw(0..3, 0..1);
        }

        self.smaa_pass.record(
            &self.device,
            &self.queue,
            encoder,
            &self.ldr_view,
            &self.smaa_out_view,
        );

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

        self.taa_prev_vp_unjittered = unjittered_vp;
    }

    fn record_standard_post(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        jittered_vp: Mat4,
        unjittered_vp: Mat4,
    ) {
        self.record_velocity_for_motion_blur(encoder, jittered_vp);
        let fx_settings = self.post_fx_settings_for_frame(encoder);
        let fx_out = self.post_fx.run_chain(
            encoder,
            &self.queue,
            ChainInput::Hdr,
            &self.chain_frame(),
            &fx_settings,
        );
        let hdr_src = self.hdr_after_post_fx(fx_out);
        self.record_bloom_chain(encoder, hdr_src);

        let tone_bg = match fx_out {
            Some(i) => &self.post_fx.tone_bgs[i],
            None => &self.tone_mapping_bind_group,
        };
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
        post_pass.set_bind_group(0, tone_bg, &[]);
        post_pass.draw(0..3, 0..1);

        self.taa_prev_vp_unjittered = unjittered_vp;
    }
}
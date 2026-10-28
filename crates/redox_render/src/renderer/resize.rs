//! Swapchain / internal render-target resizing.

use super::*;

impl Renderer {
    /// Reconfigure the surface and depth buffer after a window resize.
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width == 0 || new_height == 0 {
            return;
        }
        self.config.width = new_width;
        self.config.height = new_height;
        if let Some(ref surface) = self.surface {
            surface.configure(&self.device, &self.config);
        }

        // Recompute internal resolution (TAAU input size)
        self.internal_width = ((new_width as f32) * self.render_scale).round().max(1.0) as u32;
        self.internal_height = ((new_height as f32) * self.render_scale).round().max(1.0) as u32;

        if self.reactive_texture.is_some() {
            self.ensure_reactive_texture();
        }

        // Depth (internal)
        let (dt, dv) =
            create_depth_texture(&self.device, self.internal_width, self.internal_height);
        self.depth_texture = dt;
        self.depth_view = dv;
        let (pdt, pdv) = create_prev_depth_texture(
            &self.device,
            &self.queue,
            self.internal_width,
            self.internal_height,
        );
        self.prev_depth_texture = pdt;
        self.prev_depth_view = pdv;

        // HDR (internal)
        let (ht, hv, hs) =
            create_hdr_texture(&self.device, self.internal_width, self.internal_height);
        self.hdr_texture = ht;
        self.hdr_view = hv;
        self.hdr_sampler = hs;

        if self.msaa_sample_count > 1 {
            let (mt, mv) = create_hdr_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.hdr_msaa_texture = mt;
            self.hdr_msaa_view = mv;
            let (dmt, dmv) = create_depth_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.depth_msaa_texture = dmt;
            self.depth_msaa_view = dmv;
            self.msaa_resolve_bg = self
                .msaa_resolve_pass
                .create_bind_group(&self.device, &self.hdr_msaa_view);
        }

        // Sharp AA intermediates (output resolution)
        let (lt, lv) = create_sharp_intermediate(&self.device, new_width, new_height, "sharp_ldr");
        self.ldr_texture = lt;
        self.ldr_view = lv;
        let (st, sv) =
            create_sharp_intermediate(&self.device, new_width, new_height, "sharp_smaa_out");
        self.smaa_out_texture = st;
        self.smaa_out_view = sv;
        self.smaa_pass.resize(&self.device, new_width, new_height);

        // Tone Mapping Bind Group (samples from the new HDR texture).
        self.tone_mapping_bind_group = create_tonemap_bind_group(
            &self.device,
            &self.tone_mapping_pipeline,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_uniform_buffer,
        );

        // Normal + GTAO textures (half-res compute, full-res upsample)
        let half_width = (self.internal_width / 2).max(1);
        let half_height = (self.internal_height / 2).max(1);
        let gtao_fmt = wgpu::TextureFormat::Rgba16Float;

        let (nt, nv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::Rgba16Float,
            "normal",
        );
        self.normal_texture = nt;
        self.normal_view = nv;

        let (sr, sv) =
            create_normal_ssao_texture(&self.device, half_width, half_height, gtao_fmt, "gtao_raw");
        self.ssao_raw_texture = sr;
        self.ssao_raw_view = sv;

        let (sh, shv) = create_normal_ssao_texture(
            &self.device,
            half_width,
            half_height,
            gtao_fmt,
            "gtao_half_blurred",
        );
        self.ssao_half_blurred_texture = sh;
        self.ssao_half_blurred_view = shv;

        let (sb, sbv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            gtao_fmt,
            "gtao_blurred",
        );
        self.ssao_blurred_texture = sb;
        self.ssao_blurred_view = sbv;

        let (vsr, vsv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_raw",
        );
        self.vsm_shadow_raw_texture = vsr;
        self.vsm_shadow_raw_view = vsv;
        let (vsf, vsfv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_filtered",
        );
        self.vsm_shadow_filtered_texture = vsf;
        self.vsm_shadow_filtered_view = vsfv;
        let (vsh, vshv) = create_normal_ssao_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::R16Float,
            "vsm_shadow_history",
        );
        self.vsm_shadow_history_texture = vsh;
        self.vsm_shadow_history_view = vshv;

        self.normal_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("normal_bind_group"),
            layout: &self.normal_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.model_buffer.as_entire_binding(),
                },
            ],
        });

        let ssao_sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ssao_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        self.ssao_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao_bind_group"),
            layout: &self.ssao_pass.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &self.ssao_pass.noise_texture.view,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&ssao_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.camera_buffer.as_entire_binding(),
                },
            ],
        });

        self.ssao_blur_bind_group = self.ssao_pass.create_blur_bind_group(
            &self.device,
            &self.ssao_raw_view,
            &self.common_sampler,
            &self.normal_view,
        );
        self.ssao_upsample_bind_group = self.ssao_pass.create_upsample_bind_group(
            &self.device,
            &self.ssao_half_blurred_view,
            &self.common_sampler,
            &self.normal_view,
        );

        self.vsm_shadow_trace_bind_group = self.vsm_shadow_pass.create_trace_bind_group(
            &self.device,
            &self.normal_view,
            &self.camera_buffer,
            &self.light_buffer,
            &self.vsm_pages_array_view,
            &self.vsm_sampler,
            &self.vsm_info_buffer,
            &self.blue_noise_view,
            &self.blue_noise_sampler,
        );
        self.vsm_shadow_temporal_bind_group = self.vsm_shadow_pass.create_temporal_bind_group(
            &self.device,
            &self.vsm_shadow_raw_view,
            &self.vsm_shadow_history_view,
            &self.normal_view,
            &self.camera_buffer,
            &self.common_sampler,
        );

        // Rebuild cluster buffers for the new internal dimensions so that the
        // lighting grid matches the shading resolution.
        self.hiz
            .resize(&self.device, self.internal_width, self.internal_height);

        self.cluster_manager.resize_for_screen(
            self.internal_width,
            self.internal_height,
            &self.device,
            &self.queue,
        );

        self.update_global_bind_group();

        // Recreate TAA history at output resolution (full-res)
        let (vh0, vv0) =
            create_history_texture(&self.device, new_width, new_height, "taa_history_0");
        let (vh1, vv1) =
            create_history_texture(&self.device, new_width, new_height, "taa_history_1");
        self.taa_history_tex = [vh0, vh1];
        self.taa_history_view = [vv0, vv1];
        // Velocity is generated at internal resolution.
        let (vt, vv) =
            create_velocity_texture(&self.device, self.internal_width, self.internal_height);
        self.taa_velocity_tex = vt;
        self.taa_velocity_view = vv;
        // Force history reset so stale data is not blended on the next frame
        self.taa_frame_count = 0;
        self.taa_history_read_idx = 0;
        self.reset_vsm_shadow_temporal();
        // Rebuild cached per-frame bind groups against the new textures.
        self.velocity_bg = self
            .velocity_pass
            .create_bind_group(&self.device, &self.depth_view);
        // Bind real reactive mask (not 1×1 fallback) after HDR/velocity recreate.
        self.rebuild_taa_history_bind_groups();
        let make_tone = |view: &wgpu::TextureView| {
            create_tonemap_bind_group(
                &self.device,
                &self.tone_mapping_pipeline,
                view,
                &self.bloom_mip_views[0],
                &self.hdr_sampler,
                &self.tonemap_uniform_buffer,
            )
        };
        self.taa_tone_bgs = [
            make_tone(&self.taa_history_view[0]),
            make_tone(&self.taa_history_view[1]),
        ];

        self.vg_vis_targets =
            create_vg_vis_targets(&self.device, self.internal_width, self.internal_height);
        self.rebuild_vg_bind_groups();

        let (bloom_mip_textures, bloom_mip_views) = create_bloom_mip_chain(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::Rgba16Float,
        );
        let (bst, bsv) = create_bloom_texture(
            &self.device,
            self.internal_width,
            self.internal_height,
            wgpu::TextureFormat::Rgba16Float,
            "bloom_scratch",
        );
        self.bloom_mip_textures = bloom_mip_textures;
        self.bloom_mip_views = bloom_mip_views;
        self.bloom_scratch_texture = bst;
        self.bloom_scratch_view = bsv;
        self.rebuild_bloom_bind_groups();
        // The tone-mapping bind groups above were built against the old bloom texture, which would
        // keep showing the last pre-resize bloom forever; bind the new one.
        self.tone_mapping_bind_group = create_tonemap_bind_group(
            &self.device,
            &self.tone_mapping_pipeline,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_uniform_buffer,
        );
        let make_tone = |view: &wgpu::TextureView| {
            create_tonemap_bind_group(
                &self.device,
                &self.tone_mapping_pipeline,
                view,
                &self.bloom_mip_views[0],
                &self.hdr_sampler,
                &self.tonemap_uniform_buffer,
            )
        };
        self.taa_tone_bgs = [
            make_tone(&self.taa_history_view[0]),
            make_tone(&self.taa_history_view[1]),
        ];
        self.post_fx.rebuild(
            &self.device,
            new_width,
            new_height,
            &PostFxInputs {
                history_views: [&self.taa_history_view[0], &self.taa_history_view[1]],
                velocity_view: &self.taa_velocity_view,
                depth_view: &self.depth_view,
                bloom_view: &self.bloom_mip_views[0],
                tone_pipeline: &self.tone_mapping_pipeline,
                tone_uniform: &self.tonemap_uniform_buffer,
                sampler: &self.hdr_sampler,
            },
        );
        self.rebuild_sharp_bind_groups();

        log::info!("Resized to {}×{}", new_width, new_height);
    }

    pub(super) fn rebuild_bloom_bind_groups(&mut self) {
        self.bloom_down_bgs = create_bloom_downsample_bgs(
            &self.bloom_pass,
            &self.device,
            &self.hdr_view,
            &self.bloom_mip_views,
            &self.hdr_sampler,
        );
        self.bloom_up_bgs = create_bloom_upsample_bgs(
            &self.bloom_pass,
            &self.device,
            &self.bloom_mip_views,
            &self.hdr_sampler,
        );
        self.bloom_composite_bg = self.bloom_pass.create_composite_bind_group(
            &self.device,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
        );
        self.scatter_light_bind_group = self.scatter_light_pass.create_bind_group(
            &self.device,
            &self.hdr_view,
            &self.depth_view,
            &self.hdr_sampler,
            &self.cluster_manager.point_lights_buffer,
        );
    }
}

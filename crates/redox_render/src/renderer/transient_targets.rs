//! Rebuilds the transient texture pool and binds logical views on the renderer.

use super::*;
use crate::frame_graph::{self, FrameFeatures};
use crate::transient_pool::PoolKey;

impl Renderer {
    /// Features that control optional passes in the frame graph declaration.
    pub fn frame_features(&self) -> FrameFeatures {
        FrameFeatures {
            fog: self.post_fx_settings.fog.is_some(),
        }
    }

    /// Recompiles the frame graph and (re)allocates pooled transients for the current size.
    pub fn rebuild_transient_pool(&mut self) {
        let features = self.frame_features();
        let graph = frame_graph::frame_graph_with_features(
            self.internal_width,
            self.internal_height,
            features,
        )
        .compile()
        .expect("frame graph is valid");
        let key = PoolKey {
            width: self.internal_width,
            height: self.internal_height,
            features,
        };
        self.transient_pool.rebuild(&self.device, &graph, key);
        self.bind_transient_views();
        self.rebind_pool_dependent_resources();
        self.compiled_frame_graph = graph.clone();
    }

    fn rebind_pool_dependent_resources(&mut self) {
        // The pool just replaced hdr / depth / normals. Every group built earlier in init or
        // resize still names the previous textures, so TAA and the post chain sampled empty ones.
        self.velocity_bg = self
            .velocity_pass
            .create_bind_group(&self.device, &self.depth_view);
        self.rebuild_bloom_bind_groups();
        self.rebuild_taa_history_bind_groups();
        self.tone_mapping_bind_group = create_tonemap_bind_group(
            &self.device,
            &self.tone_mapping_pipeline,
            &self.hdr_view,
            &self.bloom_mip_views[0],
            &self.hdr_sampler,
            &self.tonemap_uniform_buffer,
        );
        self.rebuild_sharp_bind_groups();
        self.rebind_post_fx();
        self.rebind_normal_consumers();
        // The VG resolve reads the main depth: rebuild it with the pool's depth view too, or it
        // samples the pre-swap depth (all 1.0) and discards every VG pixel.
        self.vg_bind_groups_dirty = true;
        let hdr = self.hdr_texture.clone();
        let depth = self.depth_texture.clone();
        self.frame_debug.note_bindings(&hdr, &depth);
    }

    /// Post chain groups (SSR, motion blur, fog) over the textures the pool now owns.
    fn rebind_post_fx(&mut self) {
        let hdr = self.hdr_view.clone();
        let depth = self.depth_view.clone();
        let normal = self.normal_view.clone();
        let history = [
            self.taa_history_view[0].clone(),
            self.taa_history_view[1].clone(),
        ];
        let velocity = self.taa_velocity_view.clone();
        let surface = self.material_surface_view.clone();
        let reflect = self.gi_reflect.view().clone();
        let bloom = self.bloom_mip_views[0].clone();
        let vsm_pages = self.vsm_pages_array_view.clone();
        self.post_fx.rebuild(
            &self.device,
            self.config.width,
            self.config.height,
            &PostFxInputs {
                hdr_view: &hdr,
                history_views: [&history[0], &history[1]],
                velocity_view: &velocity,
                depth_view: &depth,
                normal_view: &normal,
                surface_view: &surface,
                reflect_view: &reflect,
                bloom_view: &bloom,
                tone_pipeline: &self.tone_mapping_pipeline,
                tone_uniform: &self.tonemap_uniform_buffer,
                sampler: &self.hdr_sampler,
                vsm_info: &self.vsm_info_buffer,
                vsm_pages: &vsm_pages,
            },
        );
    }

    /// GTAO and the VSM screen trace sample the normal target the pool just replaced.
    fn rebind_normal_consumers(&mut self) {
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
    }

    fn bind_transient_views(&mut self) {
        if let Some(tex) = self.transient_pool.texture("depth") {
            self.depth_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("depth") {
            self.depth_view = view.clone();
        }
        if let Some(tex) = self.transient_pool.texture("hdr_color") {
            self.hdr_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("hdr_color") {
            self.hdr_view = view.clone();
        }
        if let Some(tex) = self.transient_pool.texture("normals") {
            self.normal_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("normals") {
            self.normal_view = view.clone();
        }
        if let Some(tex) = self.transient_pool.texture("ao") {
            self.ssao_blurred_texture = tex.clone();
        }
        if let Some(view) = self.transient_pool.view("ao") {
            self.ssao_blurred_view = view.clone();
        }
    }

    /// Frame graph report at the current internal resolution (includes aliased savings).
    pub fn frame_graph_report_with_pool(&self) -> String {
        let features = self.frame_features();
        let mut out = frame_graph::frame_graph_report_with_features(
            self.internal_width,
            self.internal_height,
            features,
        );
        let saved = self
            .transient_pool
            .aliased_saved_bytes(&self.compiled_frame_graph);
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("pool_aliased_saved_bytes: {saved}\n"),
        );
        out
    }
}

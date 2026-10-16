//! Geometry/HDR attachment selection and MSAA-dependent pipeline rebuilds.

use super::*;

impl Renderer {
    /// Which HDR geometry pass performs the MSAA resolve (when `msaa_sample_count > 1`).
    pub(super) fn hdr_geometry_resolve_pass(
        &self,
        objects: &[RenderObject],
        visibility: &[bool],
    ) -> HdrGeometryResolvePass {
        if self.aa_sharp && self.msaa_sample_count > 1 {
            return HdrGeometryResolvePass::SmartResolve;
        }
        if self.has_alpha_objects(objects, visibility) {
            HdrGeometryResolvePass::Transparent
        } else if self.vg_use_vis_path
            && self
                .vg_system
                .as_ref()
                .map(|v| v.has_content())
                .unwrap_or(false)
        {
            HdrGeometryResolvePass::VgResolve
        } else if self
            .vg_system
            .as_ref()
            .map(|v| v.has_content())
            .unwrap_or(false)
        {
            HdrGeometryResolvePass::VgDebug
        } else {
            HdrGeometryResolvePass::Forward
        }
    }

    pub(super) fn geometry_depth_view(&self) -> &wgpu::TextureView {
        if self.msaa_sample_count > 1 {
            &self.depth_msaa_view
        } else {
            &self.depth_view
        }
    }

    pub(super) fn geometry_depth_attachment<'a>(
        &'a self,
        load: wgpu::LoadOp<f32>,
    ) -> wgpu::RenderPassDepthStencilAttachment<'a> {
        wgpu::RenderPassDepthStencilAttachment {
            view: self.geometry_depth_view(),
            depth_ops: Some(wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }
    }

    pub(super) fn hdr_color_attachment<'a>(
        hdr_view: &'a wgpu::TextureView,
        hdr_msaa_view: &'a wgpu::TextureView,
        msaa_sample_count: u32,
        load: wgpu::LoadOp<wgpu::Color>,
        resolve_pass: HdrGeometryResolvePass,
        this_pass: HdrGeometryResolvePass,
    ) -> wgpu::RenderPassColorAttachment<'a> {
        let resolve = msaa_sample_count > 1 && resolve_pass == this_pass;
        if msaa_sample_count > 1 {
            wgpu::RenderPassColorAttachment {
                view: hdr_msaa_view,
                resolve_target: if resolve { Some(hdr_view) } else { None },
                ops: wgpu::Operations {
                    load,
                    store: if resolve {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            }
        } else {
            wgpu::RenderPassColorAttachment {
                view: hdr_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            }
        }
    }

    pub(super) fn sync_msaa_sample_count(&mut self) {
        let new_count = if self.aa_sharp || self.taa_enabled {
            self.hdr_msaa_target_count
        } else {
            1
        };
        if new_count == self.msaa_sample_count {
            return;
        }
        self.msaa_sample_count = new_count;
        self.recreate_geometry_pipelines();
        if self.msaa_sample_count > 1 {
            let (mt, mv) = create_hdr_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.hdr_msaa_texture = mt;
            self.hdr_msaa_view = mv;
            let (dt, dv) = create_depth_msaa_texture(
                &self.device,
                self.internal_width,
                self.internal_height,
                self.msaa_sample_count,
            );
            self.depth_msaa_texture = dt;
            self.depth_msaa_view = dv;
            self.msaa_resolve_pass
                .set_sample_count(&self.queue, self.msaa_sample_count);
            self.msaa_resolve_bg = self
                .msaa_resolve_pass
                .create_bind_group(&self.device, &self.hdr_msaa_view);
        }
    }

    pub(super) fn recreate_geometry_pipelines(&mut self) {
        let hdr_format = wgpu::TextureFormat::Rgba16Float;
        let sc = self.msaa_sample_count;
        self.pbr_pass = PbrPass::new(&self.device, hdr_format, sc);
        self.transparent_pass = TransparentPass::new(&self.device, &self.pbr_pass, hdr_format, sc);
        self.debug_lines_pass =
            DebugLinesPass::new(&self.device, &self.shared_layouts.camera_bgl, sc);
        self.vg_resolve_pass = VgResolvePass::new(
            &self.device,
            hdr_format,
            &self.pbr_pass.frame_bgl,
            &self.pbr_pass.scene_bgl,
            sc,
        );
        if let Some(sb) = &mut self.skybox_pass {
            *sb = SkyboxPass::new(
                &self.device,
                &self.camera_buffer,
                &self.ibl_resource.environment_view,
                &self.ibl_processor.sampler,
                hdr_format,
                sc,
            );
        }
        self.update_global_bind_group();
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
        }
    }

    // ── TAA helpers ─────────────────────────────────────────────────────────
}

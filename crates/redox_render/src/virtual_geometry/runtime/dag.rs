//! GPU-cull recording of the VG runtime: the two-phase Hi-Z cull (phase 1 on the previous pyramid,
//! phase 2 re-tests the rejects against the current one) and the readback of the draw count. The
//! flat meshlet path and the DAG path (see `dag_gpu`) share this recording.

use crate::virtual_geometry::culling::Frustum;
use crate::virtual_geometry::gpu_cull::CullParamsUniform;

use super::VGSystem;

impl VGSystem {
    /// Poll deferred GPU-cull readback (uses last frame's count for draws).
    pub fn poll_gpu_cull_count(&mut self, device: &wgpu::Device) {
        self.gpu_cull.poll_readback(device, &mut self.indirect);
    }

    /// Record GPU cull into the main frame encoder (no CPU stall).
    pub fn record_gpu_cull(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
    ) {
        self.upload_meshlets_if_dirty(queue);
        let params = self.cull_params(view_proj, camera_pos, hiz, 0);
        let inst_count = self.instance_data.len() as u32;
        self.gpu_cull.record_dispatch(
            device,
            encoder,
            queue,
            &params,
            &self.instance_buffer,
            &self.meshlet_gpu_buffer,
            &self.indirect,
            inst_count,
            hiz.view(0),
            hiz.sampler(),
            true,
        );
    }

    /// Phase 2 late cull: re-test Hi-Z rejects against the current-frame pyramid.
    pub fn record_gpu_cull_phase2(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
    ) {
        if !self.config.gpu_cull || self.active_instance_count() == 0 {
            return;
        }
        let params = self.cull_params(view_proj, camera_pos, hiz, 1);
        let inst_count = self.instance_data.len() as u32;
        self.gpu_cull.record_dispatch(
            device,
            encoder,
            queue,
            &params,
            &self.instance_buffer,
            &self.meshlet_gpu_buffer,
            &self.indirect,
            inst_count,
            hiz.view(0),
            hiz.sampler(),
            false,
        );
    }

    fn cull_params(
        &self,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
        cull_mode: u32,
    ) -> CullParamsUniform {
        let frustum = Frustum::from_view_proj(view_proj);
        CullParamsUniform {
            planes: frustum.planes,
            camera_pos: [camera_pos.x, camera_pos.y, camera_pos.z, 1.0],
            view_proj: *view_proj,
            hiz_size: [hiz.width as f32, hiz.height as f32],
            hiz_mip_count: hiz.mip_count,
            hiz_enabled: u32::from(hiz.is_ready()),
            cull_mode,
            reject_capacity: self.gpu_cull.reject_capacity(),
            _pad: [0; 2],
        }
    }

    /// Schedule async count readback on the main encoder.
    pub fn schedule_gpu_cull_readback(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.gpu_cull.schedule_readback(encoder);
    }

    /// Begin async map after submit.
    pub fn kick_gpu_cull_readback(&mut self) {
        self.gpu_cull.kick_readback();
    }

    /// Run GPU cull (requires [`VGConfig::gpu_cull`] and a pre-built encoder).
    #[allow(dead_code)]
    pub fn run_gpu_cull(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        hiz: &crate::virtual_geometry::hiz::HiZPyramid,
    ) -> u32 {
        self.upload_meshlets_if_dirty(queue);
        let params = self.cull_params(view_proj, camera_pos, hiz, 0);
        let inst_count = self.instance_data.len() as u32;
        let mut enc = encoder;
        self.gpu_cull.record_dispatch(
            device,
            &mut enc,
            queue,
            &params,
            &self.instance_buffer,
            &self.meshlet_gpu_buffer,
            &self.indirect,
            inst_count,
            hiz.view(0),
            hiz.sampler(),
            true,
        );
        let count = self
            .gpu_cull
            .read_command_count(device, queue, enc, &mut self.indirect);
        self.stats.visible_meshlets = count;
        count
    }
}

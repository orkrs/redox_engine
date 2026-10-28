//! GPU memory accounting built from the live resources.

use super::*;
use crate::memory_budget::{MemoryBudget, MemoryCategory, texture_bytes_blocks};

/// Bytes a wgpu texture occupies (all mips, layers, samples; block compressed formats by block).
fn gpu_texture_bytes(texture: &wgpu::Texture) -> u64 {
    let format = texture.format();
    let (block_width, block_height) = format.block_dimensions();
    let block_bytes = format.block_copy_size(None).unwrap_or(0);
    texture_bytes_blocks(
        (
            texture.width(),
            texture.height(),
            texture.depth_or_array_layers(),
        ),
        texture.mip_level_count(),
        (block_width, block_height, block_bytes),
        texture.sample_count(),
        texture.dimension() == wgpu::TextureDimension::D3,
    )
}

impl Renderer {
    /// Sets or clears the byte limit of a memory category. Limits do not stop uploads; they only mark
    /// the category as over budget in [`Self::memory_usage`] and [`Self::memory_report`].
    pub fn set_memory_limit(&mut self, category: MemoryCategory, bytes: Option<u64>) {
        self.memory_limits.set_limit(category, bytes);
    }

    /// GPU memory by category, computed from the resources that exist right now (so unloading is
    /// always reflected), with the configured limits attached.
    ///
    /// Textures and meshes come from the registry, shadows from the local atlas and the VSM
    /// textures, virtual geometry from its big buffers, `Other` from the render targets, and
    /// `Buffers` from the renderer's persistent uniform and storage buffers.
    pub fn memory_usage(&self) -> MemoryBudget {
        let mut budget = self.memory_limits.clone();
        budget.add(MemoryCategory::Meshes, self.registry.mesh_bytes());
        for texture in self.registry.textures() {
            budget.add(MemoryCategory::Textures, gpu_texture_bytes(texture));
        }
        let shadows = [
            &self.local_shadow_atlas,
            &self.vsm_pages_tex,
            &self.vsm_shadow_raw_texture,
            &self.vsm_shadow_filtered_texture,
            &self.vsm_shadow_history_texture,
        ];
        for texture in shadows {
            budget.add(MemoryCategory::Shadows, gpu_texture_bytes(texture));
        }
        if let Some(vg) = &self.vg_system {
            budget.add(MemoryCategory::VirtualGeometry, vg.gpu_bytes());
        }
        let targets = [
            &self.hdr_texture,
            &self.hdr_msaa_texture,
            &self.normal_texture,
            &self.ssao_raw_texture,
            &self.ssao_half_blurred_texture,
            &self.ssao_blurred_texture,
            &self.depth_texture,
            &self.depth_msaa_texture,
            &self.prev_depth_texture,
            &self.ldr_texture,
            &self.smaa_out_texture,
            &self.bloom_scratch_texture,
            &self.taa_velocity_tex,
        ];
        for texture in targets {
            budget.add(MemoryCategory::Other, gpu_texture_bytes(texture));
        }
        // Not counted here: buffers owned by subsystems: cluster_manager (point lights, metadata, light indices, bounds, info),
        // registry material uniforms, skinning_gpu skin buffer, the taa and velocity uniforms, and
        // the per-pass buffers under pass/, post/, gi/, sdf/, shadow/ and virtual_geometry/.
        let buffers = [
            &self.camera_buffer,
            &self.light_buffer,
            &self.shader_debug_buffer,
            &self.tonemap_uniform_buffer,
            &self.model_buffer,
            &self.tonemap_rcas_uniform_buffer,
            &self.prev_model_buffer,
            &self.vsm_pageinfo_buffer,
            &self.vsm_info_buffer,
            &self.vg_material_buffer,
        ];
        for buffer in buffers.into_iter().chain(&self.face_matrix_buffers) {
            budget.add(MemoryCategory::Buffers, buffer.size());
        }
        budget
    }

    /// Text report of [`Self::memory_usage`] for a debug overlay (see [`MemoryBudget::report`]).
    pub fn memory_report(&self) -> String {
        self.memory_usage().report()
    }
}

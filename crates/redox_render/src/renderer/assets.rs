//! Mesh / texture / material upload and asset-handle lookup.

use super::*;
use crate::mesh::Vertex;

impl Renderer {
    /// Uploads a CPU [`Mesh`] to the GPU and returns its slot index.
    pub fn upload_mesh(&mut self, mesh: &Mesh) -> usize {
        let vertex_bytes: Vec<u8> = if self.use_compressed_vertices {
            let compressed = compress_vertices(&mesh.vertices, &mesh.aabb);
            bytemuck::cast_slice(&compressed).to_vec()
        } else {
            bytemuck::cast_slice(&mesh.vertices).to_vec()
        };
        let vb = buffer::create_vertex_buffer(&self.device, "mesh_vb", &vertex_bytes);
        let ib = buffer::create_index_buffer(
            &self.device,
            "mesh_ib",
            bytemuck::cast_slice(&mesh.indices),
        );
        self.registry.insert_mesh_slot(GpuMesh {
            vertex_buffer: vb,
            index_buffer: ib,
            index_count: mesh.index_count(),
            local_bounds: mesh.aabb,
        })
    }

    /// Overwrites the vertices of an uploaded mesh in place (skinning, morphing).
    ///
    /// Positions are quantised against the bounds the mesh was uploaded with, so deformed
    /// vertices must stay inside them. Returns `false` (and writes nothing) when `slot` is
    /// unknown or `vertices` has a different length than the uploaded mesh.
    pub fn update_mesh_vertices(&self, slot: usize, vertices: &[Vertex]) -> bool {
        let Some(gpu) = self.registry.mesh_by_slot(slot) else {
            return false;
        };
        let bytes: Vec<u8> = if self.use_compressed_vertices {
            bytemuck::cast_slice(&compress_vertices(vertices, &gpu.local_bounds)).to_vec()
        } else {
            bytemuck::cast_slice(vertices).to_vec()
        };
        if bytes.len() as u64 != gpu.vertex_buffer.size() {
            return false;
        }
        self.queue.write_buffer(&gpu.vertex_buffer, 0, &bytes);
        true
    }

    /// Stores a material, creates its GPU resources, and returns its index.
    pub fn add_material(&mut self, material: Material) -> usize {
        let uniform = material.to_uniform(&self.device);
        let buffer = buffer::create_uniform_buffer(
            &self.device,
            "material_uniform",
            bytemuck::bytes_of(&uniform),
        );

        let albedo_view = material
            .texture_index
            .and_then(|idx| self.registry.legacy_texture(idx))
            .map(|t| &t.view)
            .unwrap_or_else(|| {
                self.registry
                    .legacy_texture(0)
                    .map(|t| &t.view)
                    .expect("fallback texture")
            });

        let normal_view = material
            .normal_texture_index
            .and_then(|idx| self.registry.legacy_texture(idx))
            .map(|t| &t.view)
            .unwrap_or(&self.fallback_normal_view);

        let mr_view = material
            .mr_texture_index
            .and_then(|idx| self.registry.legacy_texture(idx))
            .map(|t| &t.view)
            .unwrap_or(&self.fallback_mr_view);

        let bind_group = self.pbr_pass.create_material_bind_group(
            &self.device,
            &buffer,
            albedo_view,
            normal_view,
            mr_view,
            &self.common_sampler,
        );

        let idx = self
            .registry
            .push_legacy_material(material, buffer, bind_group);
        self.material_blend_flags.push(BlendMode::Opaque);
        self.vg_bind_groups_dirty = true;
        idx
    }

    /// Uploads a texture from raw bytes and returns its index.
    pub fn upload_texture(
        &mut self,
        bytes: &[u8],
        label: &str,
    ) -> Result<usize, image::ImageError> {
        let tex = Texture::from_bytes(
            &self.device,
            &self.queue,
            bytes,
            label,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )?;
        let bg = self
            .shared_layouts
            .create_texture_bind_group(&self.device, &tex);
        Ok(self.registry.push_legacy_texture(tex, bg))
    }

    /// Uploads a texture with specific format (e.g. for normals/MR).
    pub fn upload_texture_with_format(
        &mut self,
        bytes: &[u8],
        label: &str,
        format: wgpu::TextureFormat,
    ) -> Result<usize, image::ImageError> {
        let tex = Texture::from_bytes(&self.device, &self.queue, bytes, label, format)?;
        let bg = self
            .shared_layouts
            .create_texture_bind_group(&self.device, &tex);
        Ok(self.registry.push_legacy_texture(tex, bg))
    }

    // ----- Asset-handle–based upload (for redox_asset integration) -----

    /// Mesh slot of a mesh asset handle.
    pub fn get_mesh_index(&self, handle: Handle<MeshData>) -> Option<usize> {
        self.registry.mesh_index_for(handle.id())
    }

    /// Bounding box of the mesh in a slot.
    pub fn get_mesh_bounds(&self, slot: usize) -> Option<redox_math::Aabb> {
        self.registry.mesh_by_slot(slot).map(|m| m.local_bounds)
    }

    /// Texture slot of a texture asset handle.
    pub fn get_texture_index(&self, handle: Handle<TextureData>) -> Option<usize> {
        self.registry.texture_index_for(handle.id())
    }

    /// Material slot of a material asset handle.
    pub fn get_material_index(&self, handle: Handle<MaterialData>) -> Option<usize> {
        self.registry.material_index_for(handle.id())
    }

    /// Uploads a mesh asset and returns its slot.
    pub fn add_mesh_from_asset(&mut self, handle: Handle<MeshData>, mesh: &Mesh) -> Option<usize> {
        if self.registry.has_mesh(handle.id()) {
            return self.registry.mesh_index_for(handle.id());
        }
        let gpu_mesh = {
            let vb = buffer::create_vertex_buffer(
                &self.device,
                "mesh_vb",
                bytemuck::cast_slice(&mesh.vertices),
            );
            let ib = buffer::create_index_buffer(
                &self.device,
                "mesh_ib",
                bytemuck::cast_slice(&mesh.indices),
            );
            GpuMesh {
                vertex_buffer: vb,
                index_buffer: ib,
                index_count: mesh.index_count(),
                local_bounds: mesh.aabb,
            }
        };
        Some(self.registry.insert_mesh(handle.id(), gpu_mesh))
    }

    /// Uploads a texture asset and returns its slot.
    pub fn add_texture_from_asset(
        &mut self,
        handle: Handle<TextureData>,
        img: &image::DynamicImage,
        label: &str,
    ) -> Result<Option<usize>, image::ImageError> {
        if self.registry.has_texture(handle.id()) {
            return Ok(self.registry.texture_index_for(handle.id()));
        }
        let tex = Texture::from_image(
            &self.device,
            &self.queue,
            img,
            label,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )?;
        let bg = self
            .shared_layouts
            .create_texture_bind_group(&self.device, &tex);
        let idx = self.registry.insert_texture(
            handle.id(),
            crate::registry::TextureSlot {
                texture: tex,
                bind_group: Some(bg),
            },
        );
        Ok(Some(idx))
    }

    /// Uploads a material asset and returns its slot.
    pub fn add_material_from_asset(
        &mut self,
        handle: Handle<MaterialData>,
        data: &MaterialData,
    ) -> Option<usize> {
        if self.registry.has_material(handle.id()) {
            return self.registry.material_index_for(handle.id());
        }
        let texture_index = data
            .albedo_handle
            .as_ref()
            .and_then(|h| self.registry.texture_index_for(h.id()));
        let normal_texture_index = data
            .normal_handle
            .as_ref()
            .and_then(|h| self.registry.texture_index_for(h.id()));
        let mr_texture_index = data
            .mr_handle
            .as_ref()
            .and_then(|h| self.registry.texture_index_for(h.id()));
        let material = Material {
            base_color: data.base_color,
            texture_index,
            normal_texture_index,
            mr_texture_index,
            metallic: data.metallic,
            roughness: data.roughness,
            emissive: data.emissive,
            layers: data.layers,
            sheen: data.sheen,
        };
        let idx = self.add_material(material);
        self.material_blend_flags[idx] = data.blend;
        self.registry.link_material_index(handle.id(), idx);
        Some(idx)
    }

    // ----- Rendering -----
}

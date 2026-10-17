//! GPU resource registry with slot reuse.
//!
//! Replaces the append-only `Vec<GpuMesh> / Vec<Material> / Vec<Texture>`
//! storage that leaked by design. Slots are recycled through a free list, and
//! `AssetId` handles map to slot indices so removal is O(1).

use std::collections::HashMap;

use redox_asset::AssetId;

use crate::asset_types::MaterialData;
use crate::material::Material;
use crate::resource::texture::Texture;

/// A GPU mesh (vertex + index buffers).
pub struct GpuMesh {
    /// GPU vertex buffer.
    pub vertex_buffer: wgpu::Buffer,
    /// GPU index buffer.
    pub index_buffer: wgpu::Buffer,
    /// Number of indices to draw.
    pub index_count: u32,
    /// Mesh-local AABB (from CPU vertices) used for frustum culling.
    pub local_bounds: redox_math::Aabb,
}

/// Everything owned by the GPU for one uploaded material.
pub struct MaterialSlot {
    /// CPU-side material parameters mirrored for rebuilds.
    pub data: MaterialData,
    /// Uniform buffer holding the material parameters.
    pub uniform_buffer: wgpu::Buffer,
    /// Bind group with the uniform buffer and textures.
    pub bind_group: wgpu::BindGroup,
}

/// Everything owned by the GPU for one uploaded texture.
pub struct TextureSlot {
    /// The GPU texture with its view and sampler.
    pub texture: Texture,
    /// Bind group sampling this texture (created at upload time).
    pub bind_group: Option<wgpu::BindGroup>,
}

/// Generic dense slot storage with free-list recycling.
struct Pool<T> {
    slots: Vec<Option<T>>,
    free: Vec<usize>,
}

impl<T> Pool<T> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn insert(&mut self, value: T) -> usize {
        match self.free.pop() {
            Some(idx) => {
                self.slots[idx] = Some(value);
                idx
            }
            None => {
                self.slots.push(Some(value));
                self.slots.len() - 1
            }
        }
    }

    fn remove(&mut self, idx: usize) -> Option<T> {
        if idx >= self.slots.len() {
            return None;
        }
        let taken = self.slots[idx].take();
        if taken.is_some() {
            self.free.push(idx);
        }
        taken
    }

    fn get(&self, idx: usize) -> Option<&T> {
        self.slots.get(idx).and_then(|s| s.as_ref())
    }

    fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        self.slots.get_mut(idx).and_then(|s| s.as_mut())
    }

    fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry of all GPU-side meshes, materials, textures and their handle maps.
///
/// Access pattern mirrors the previous `RenderContext` fields (`meshes`,
/// `handle_to_mesh_index`, …) but adds removal. `RenderContext` delegates to it.
#[derive(Default)]
pub struct GpuRegistry {
    meshes: Pool<GpuMesh>,
    handle_to_mesh: HashMap<AssetId, usize>,

    materials: Pool<MaterialSlot>,
    handle_to_material: HashMap<AssetId, usize>,
    /// Legacy index-based material storage used while examples still pass raw
    /// indices; removed when callers migrate to handles.
    pub legacy_materials: Vec<Material>,
    /// Bind groups of materials created through the legacy index-based API.
    pub legacy_material_bind_groups: Vec<wgpu::BindGroup>,
    /// Uniform buffers behind the legacy material bind groups.
    pub legacy_material_uniform_buffers: Vec<wgpu::Buffer>,

    textures: Pool<TextureSlot>,
    handle_to_texture: HashMap<AssetId, usize>,
    /// Textures created through the legacy index-based API.
    pub legacy_textures: Vec<Texture>,
    /// Bind groups of the legacy textures.
    pub legacy_texture_bind_groups: Vec<wgpu::BindGroup>,

    /// Bind group with 1x1 fallback textures, used by materials without textures.
    pub fallback_texture_bg: Option<wgpu::BindGroup>,
}

impl GpuRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    // ── Meshes ──────────────────────────────────────────────────────────────

    /// Inserts a GPU mesh and records the handle mapping.
    pub fn insert_mesh(&mut self, handle: AssetId, mesh: GpuMesh) -> usize {
        let slot = self.meshes.insert(mesh);
        self.handle_to_mesh.insert(handle, slot);
        slot
    }

    /// Removes a mesh, freeing its slot and dropping the GPU buffers.
    pub fn remove_mesh(&mut self, handle: AssetId) -> Option<GpuMesh> {
        let slot = self.handle_to_mesh.remove(&handle)?;
        self.meshes.remove(slot)
    }

    /// GPU mesh registered for an asset handle.
    pub fn mesh_by_handle(&self, handle: AssetId) -> Option<&GpuMesh> {
        self.handle_to_mesh
            .get(&handle)
            .and_then(|&i| self.meshes.get(i))
    }

    /// GPU mesh in a slot.
    pub fn mesh_by_slot(&self, slot: usize) -> Option<&GpuMesh> {
        self.meshes.get(slot)
    }

    /// Associates a material slot index with an asset handle.
    pub fn link_material_index(&mut self, handle: AssetId, idx: usize) {
        self.handle_to_material.insert(handle, idx);
    }

    /// Mesh slot of an asset handle.
    pub fn mesh_index_for(&self, handle: AssetId) -> Option<usize> {
        self.handle_to_mesh.get(&handle).copied()
    }

    /// Texture slot of an asset handle.
    pub fn texture_index_for(&self, handle: AssetId) -> Option<usize> {
        self.handle_to_texture.get(&handle).copied()
    }

    /// Material slot of an asset handle.
    pub fn material_index_for(&self, handle: AssetId) -> Option<usize> {
        self.handle_to_material.get(&handle).copied()
    }

    /// Whether a mesh is registered for the handle.
    pub fn has_mesh(&self, handle: AssetId) -> bool {
        self.handle_to_mesh.contains_key(&handle)
    }

    /// Whether a texture is registered for the handle.
    pub fn has_texture(&self, handle: AssetId) -> bool {
        self.handle_to_texture.contains_key(&handle)
    }

    /// Whether a material is registered for the handle.
    pub fn has_material(&self, handle: AssetId) -> bool {
        self.handle_to_material.contains_key(&handle)
    }

    /// Inserts a mesh without an asset handle (examples / procedural geometry).
    pub fn insert_mesh_slot(&mut self, mesh: GpuMesh) -> usize {
        self.meshes.insert(mesh)
    }

    /// Legacy material by index.
    pub fn legacy_material(&self, idx: usize) -> Option<&Material> {
        self.legacy_materials.get(idx)
    }

    /// Bind group of a legacy material.
    pub fn legacy_material_bind_group(&self, idx: usize) -> Option<&wgpu::BindGroup> {
        self.legacy_material_bind_groups.get(idx)
    }

    /// Legacy texture by index.
    pub fn legacy_texture(&self, idx: usize) -> Option<&Texture> {
        self.legacy_textures.get(idx)
    }

    /// Bind group of a legacy texture.
    pub fn legacy_texture_bind_group(&self, idx: usize) -> Option<&wgpu::BindGroup> {
        self.legacy_texture_bind_groups.get(idx)
    }

    /// Adds a legacy material and returns its index.
    pub fn push_legacy_material(
        &mut self,
        material: Material,
        uniform_buffer: wgpu::Buffer,
        bind_group: wgpu::BindGroup,
    ) -> usize {
        let idx = self.legacy_materials.len();
        self.legacy_materials.push(material);
        self.legacy_material_uniform_buffers.push(uniform_buffer);
        self.legacy_material_bind_groups.push(bind_group);
        idx
    }

    /// Adds a legacy texture and returns its index.
    pub fn push_legacy_texture(&mut self, texture: Texture, bind_group: wgpu::BindGroup) -> usize {
        let idx = self.legacy_textures.len();
        self.legacy_textures.push(texture);
        self.legacy_texture_bind_groups.push(bind_group);
        idx
    }

    /// All legacy materials.
    pub fn legacy_materials(&self) -> &[Material] {
        &self.legacy_materials
    }

    /// All legacy material bind groups.
    pub fn legacy_material_bind_groups(&self) -> &[wgpu::BindGroup] {
        &self.legacy_material_bind_groups
    }

    /// Number of live mesh slots (for stats/debug UI).
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    // ── Materials ───────────────────────────────────────────────────────────

    /// Registers a material for a handle and returns its slot index.
    pub fn insert_material(&mut self, handle: AssetId, slot: MaterialSlot) -> usize {
        let idx = self.materials.insert(slot);
        self.handle_to_material.insert(handle, idx);
        idx
    }

    /// Removes the material registered for a handle.
    pub fn remove_material(&mut self, handle: AssetId) -> Option<MaterialSlot> {
        let idx = self.handle_to_material.remove(&handle)?;
        self.materials.remove(idx)
    }

    /// Material registered for a handle.
    pub fn material_by_handle(&self, handle: AssetId) -> Option<&MaterialSlot> {
        self.handle_to_material
            .get(&handle)
            .and_then(|&i| self.materials.get(i))
    }

    // ── Textures ────────────────────────────────────────────────────────────

    /// Registers a texture for a handle and returns its slot index.
    pub fn insert_texture(&mut self, handle: AssetId, slot: TextureSlot) -> usize {
        let idx = self.textures.insert(slot);
        self.handle_to_texture.insert(handle, idx);
        idx
    }

    /// Removes the texture registered for a handle.
    pub fn remove_texture(&mut self, handle: AssetId) -> Option<TextureSlot> {
        let idx = self.handle_to_texture.remove(&handle)?;
        self.textures.remove(idx)
    }

    /// Texture registered for a handle.
    pub fn texture_by_handle(&self, handle: AssetId) -> Option<&TextureSlot> {
        self.handle_to_texture
            .get(&handle)
            .and_then(|&i| self.textures.get(i))
    }

    /// Mutable access to the texture registered for a handle.
    pub fn texture_slot_mut(&mut self, handle: AssetId) -> Option<&mut TextureSlot> {
        let idx = *self.handle_to_texture.get(&handle)?;
        self.textures.get_mut(idx)
    }
}

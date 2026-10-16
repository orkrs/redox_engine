//! Virtual Geometry runtime: [`VGSystem`] owns all GPU resources and drives
//! the per-frame culling + rendering pipeline.
//!
//! ## Data flow (per frame)
//!
//! ```text
//!   prepare_frame():
//!     1. CPU frustum cull  →  list of DrawIndexedIndirectCmd
//!     2. Upload to IndirectDrawBuffer
//!     3. Update instance GPU buffer (if any instance changed)
//!
//!   render():
//!     1. Set vertex / index buffers from the global VG pool
//!     2. Set pipeline + bind groups
//!     3. Loop: draw_indexed_indirect per visible command
//! ```

use bytemuck;
use std::collections::HashMap;

use super::asset_pipeline::build_vg_asset;
use super::cache;
use super::culling::{Frustum, cull_meshlets_into};
use super::gpu_cull::{CullParamsUniform, VgGpuCull};
use super::indirect::IndirectDrawBuffer;
use super::lod::LodChainConfig;
use super::meshlet::{
    DrawIndexedIndirectCmd, MAX_GLOBAL_MESHLETS, MAX_VG_INSTANCES, MAX_VISIBLE_COMMANDS,
    MeshletDescriptor, VGAssetData, VGInstanceData,
};
use crate::mesh::{CompressedVertex, bounds_vectors, compress_vertices};

// ── WGSL shader source ──────────────────────────────────────────────────────

// Vertex + fragment shader for virtual geometry rendering. The source lives in
// `shaders/vg.wgsl` and is embedded at compile time (`crate::shader::VG_WGSL`).
//
// Group 0: camera uniform (binding 0); Group 1: per-instance storage buffer
// (binding 0). The fragment stage renders world-space normals tinted with a
// VG debug colour so VG objects are easy to distinguish from standard PBR
// objects; full PBR integration can be added by extending the bind groups.

// ── Config & Stats ────────────────────────────────────────────────────────────

/// Configuration for [`VGSystem`].
#[derive(Clone, Debug)]
pub struct VGConfig {
    /// Maximum total vertices across all VG assets.
    pub max_vertices: usize,
    /// Maximum total indices across all VG assets.
    pub max_indices: usize,
    /// Maximum total meshlets across all VG assets.
    pub max_meshlets: usize,
    /// Maximum active instances.
    pub max_instances: usize,
    /// Maximum visible draw commands per frame.
    pub max_visible_commands: usize,
    /// LOD chain config used when building new VG assets.
    pub lod_config: LodChainConfig,
    /// When true, culling runs on GPU via `vg_cull.wgsl` (CPU fallback otherwise).
    pub gpu_cull: bool,
}

impl Default for VGConfig {
    fn default() -> Self {
        Self {
            max_vertices: 1 << 18,
            max_indices: 1 << 19,
            max_meshlets: MAX_GLOBAL_MESHLETS.min(1 << 16),
            max_instances: MAX_VG_INSTANCES,
            max_visible_commands: MAX_VISIBLE_COMMANDS,
            lod_config: LodChainConfig::default(),
            gpu_cull: false,
        }
    }
}

/// Per-frame statistics reported by [`VGSystem`].
#[derive(Clone, Debug, Default)]
pub struct VGStats {
    pub total_meshlets: u32,
    pub visible_meshlets: u32,
    pub total_triangles: u64,
    pub visible_triangles: u64,
    pub registered_assets: u32,
    pub active_instances: u32,
}

// ── Asset handle ──────────────────────────────────────────────────────────────

/// Opaque identifier for a registered VG asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VGAssetId(pub u64);

/// Opaque handle for a spawned VG instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VGInstanceHandle(pub u32);

// ── Internal stored asset ─────────────────────────────────────────────────────

#[allow(dead_code)]
struct StoredAsset {
    meshlet_offset: u32,
    meshlet_count: u32,
    vertex_offset: u32,
    index_offset: u32,
    bounds_min: [f32; 4],
    bounds_extent: [f32; 4],
}

// ── VGSystem ──────────────────────────────────────────────────────────────────

/// Central runtime manager for virtual geometry.
///
/// Owns the global GPU vertex / index / meshlet / instance buffers and drives
/// the per-frame culling + rendering pipeline.
pub struct VGSystem {
    // GPU geometry buffers (large, allocated once)
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,

    // GPU metadata / instance buffers
    instance_buffer: wgpu::Buffer,
    meshlet_gpu_buffer: wgpu::Buffer,

    // Indirect draw
    indirect: IndirectDrawBuffer,

    // Optional GPU cull
    gpu_cull: VgGpuCull,
    /// Reused by CPU cull path each frame.
    cull_commands_scratch: Vec<DrawIndexedIndirectCmd>,

    // Pipeline (debug path — retained for diagnostics)
    pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    camera_bgl: wgpu::BindGroupLayout,
    camera_bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    instances_bgl: wgpu::BindGroupLayout,
    instances_bind_group: wgpu::BindGroup,

    // CPU-side state
    /// All meshlet descriptors accumulated from every registered asset.
    global_meshlets: Vec<MeshletDescriptor>,
    meshlets_dirty: bool,
    /// Per-instance GPU data (transforms + meshlet range).
    instance_data: Vec<VGInstanceData>,
    /// Metadata for each registered asset.
    assets: HashMap<VGAssetId, StoredAsset>,
    /// Mapping from instance handle to index in `instance_data`.
    instance_map: HashMap<VGInstanceHandle, usize>,
    next_asset_id: u64,
    next_instance_id: u32,
    vertex_watermark: u32,
    index_watermark: u32,
    instances_dirty: bool,

    pub config: VGConfig,
    pub stats: VGStats,
}

impl VGSystem {
    /// Create a new VGSystem.
    ///
    /// `camera_buffer` is the same buffer used by the main PBR pass.
    /// `surface_format` and `depth_format` must match the main render target.
    pub fn new(
        device: &wgpu::Device,
        config: VGConfig,
        camera_buffer: &wgpu::Buffer,
        surface_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        // ── GPU Geometry Buffers ──────────────────────────────────────────────
        let vb_size = (config.max_vertices * std::mem::size_of::<CompressedVertex>()) as u64;
        let ib_size = (config.max_indices * std::mem::size_of::<u32>()) as u64;

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_global_vertex_buffer"),
            size: vb_size.max(64),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_global_index_buffer"),
            size: ib_size.max(64),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Instance Buffer ───────────────────────────────────────────────────
        let inst_size = (config.max_instances * std::mem::size_of::<VGInstanceData>()) as u64;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_instance_buffer"),
            size: inst_size.max(64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let meshlet_bytes = (config.max_meshlets * std::mem::size_of::<MeshletDescriptor>()) as u64;
        let meshlet_gpu_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vg_meshlet_storage"),
            size: meshlet_bytes.max(64),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Indirect Draw Buffer ──────────────────────────────────────────────
        let indirect =
            IndirectDrawBuffer::new(device, config.max_visible_commands, "vg_indirect_draw");
        let gpu_cull = VgGpuCull::new(device);

        // ── Camera Bind Group Layout + Bind Group ─────────────────────────────
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_camera_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_camera_bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // ── Instance Bind Group Layout + Bind Group ───────────────────────────
        let instances_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vg_instances_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let instances_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vg_instances_bg"),
            layout: &instances_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instance_buffer.as_entire_binding(),
            }],
        });

        // ── Pipeline ──────────────────────────────────────────────────────────
        let pipeline = build_pipeline(
            device,
            &camera_bgl,
            &instances_bgl,
            surface_format,
            depth_format,
        );

        Self {
            vertex_buffer,
            index_buffer,
            instance_buffer,
            meshlet_gpu_buffer,
            indirect,
            gpu_cull,
            cull_commands_scratch: Vec::new(),
            pipeline,
            camera_bgl,
            camera_bind_group,
            instances_bgl,
            instances_bind_group,
            global_meshlets: Vec::new(),
            meshlets_dirty: false,
            instance_data: Vec::new(),
            assets: HashMap::new(),
            instance_map: HashMap::new(),
            next_asset_id: 0,
            next_instance_id: 0,
            vertex_watermark: 0,
            index_watermark: 0,
            instances_dirty: false,
            config,
            stats: VGStats::default(),
        }
    }

    // ── Asset registration ────────────────────────────────────────────────────

    /// Register a pre-built [`VGAssetData`] and upload it to the GPU.
    ///
    /// Returns the [`VGAssetId`] needed to spawn instances.
    pub fn register_asset(&mut self, data: VGAssetData, queue: &wgpu::Queue) -> Option<VGAssetId> {
        let vert_needed = data.vertices.len();
        let idx_needed = data.indices.len();
        let meshlet_count = data.meshlets.len() as u32;

        let max_v = self.config.max_vertices;
        let max_i = self.config.max_indices;
        if self.vertex_watermark as usize + vert_needed > max_v
            || self.index_watermark as usize + idx_needed > max_i
        {
            log::warn!(
                "[VGSystem] Not enough GPU buffer space to register asset \
                 (need {} verts, {} indices)",
                vert_needed,
                idx_needed
            );
            return None;
        }

        let vert_offset = self.vertex_watermark;
        let idx_offset = self.index_watermark;
        let meshlet_offset = self.global_meshlets.len() as u32;

        let aabb = redox_math::Aabb {
            min: redox_math::Vec3::from_array(data.aabb[0]),
            max: redox_math::Vec3::from_array(data.aabb[1]),
        };
        let compressed = compress_vertices(&data.vertices, &aabb);
        let (bounds_min, bounds_extent) = bounds_vectors(&aabb);

        queue.write_buffer(
            &self.vertex_buffer,
            (vert_offset as usize * std::mem::size_of::<CompressedVertex>()) as u64,
            bytemuck::cast_slice(&compressed),
        );

        // Upload indices
        queue.write_buffer(
            &self.index_buffer,
            (idx_offset as usize * std::mem::size_of::<u32>()) as u64,
            bytemuck::cast_slice(&data.indices),
        );

        // Accumulate meshlets into CPU mirror (used every frame for culling)
        self.global_meshlets.extend(data.meshlets);
        self.meshlets_dirty = true;

        self.vertex_watermark += vert_needed as u32;
        self.index_watermark += idx_needed as u32;

        let id = VGAssetId(self.next_asset_id);
        self.next_asset_id += 1;

        self.assets.insert(
            id,
            StoredAsset {
                meshlet_offset,
                meshlet_count,
                vertex_offset: vert_offset,
                index_offset: idx_offset,
                bounds_min,
                bounds_extent,
            },
        );

        log::info!(
            "[VGSystem] Registered VG asset {:?}: {} meshlets, {} verts, {} indices",
            id,
            meshlet_count,
            vert_needed,
            idx_needed
        );

        Some(id)
    }

    /// Build a VG asset from a standard mesh and register it.
    pub fn register_mesh(
        &mut self,
        mesh: &crate::mesh::Mesh,
        material_index: u32,
        queue: &wgpu::Queue,
    ) -> Option<VGAssetId> {
        let lod = self.config.lod_config.clone();
        let key = cache::cache_key(mesh, &lod, material_index);
        let data = cache::load_cached(key).unwrap_or_else(|| {
            let built = build_vg_asset(mesh, &lod, material_index);
            cache::store_cached(key, &built);
            built
        });
        self.register_asset(data, queue)
    }

    // ── Instance management ───────────────────────────────────────────────────

    /// Spawn a new instance of an asset in the world.
    ///
    /// `transform` must be a **column-major** 4×4 matrix (matching glam / wgpu
    /// convention).
    pub fn spawn_instance(
        &mut self,
        asset_id: VGAssetId,
        transform: [[f32; 4]; 4],
    ) -> Option<VGInstanceHandle> {
        let stored = self.assets.get(&asset_id)?;

        if self.instance_data.len() >= self.config.max_instances {
            log::warn!("[VGSystem] Max instances reached");
            return None;
        }

        let slot = self.find_free_instance_slot();
        let handle = VGInstanceHandle(self.next_instance_id);
        self.next_instance_id += 1;

        let inst = VGInstanceData {
            transform,
            meshlet_offset: stored.meshlet_offset,
            meshlet_count: stored.meshlet_count,
            flags: 0,
            _pad: 0,
            bounds_min: stored.bounds_min,
            bounds_extent: stored.bounds_extent,
        };

        if slot < self.instance_data.len() {
            self.instance_data[slot] = inst;
        } else {
            self.instance_data.push(inst);
        }

        self.instance_map.insert(handle, slot);
        self.instances_dirty = true;
        Some(handle)
    }

    /// Update the world transform of an active instance.
    pub fn update_instance_transform(
        &mut self,
        handle: VGInstanceHandle,
        transform: [[f32; 4]; 4],
    ) {
        if let Some(&slot) = self.instance_map.get(&handle) {
            self.instance_data[slot].transform = transform;
            self.instances_dirty = true;
        }
    }

    /// Remove an instance.  Its slot is reused by the next `spawn_instance`.
    pub fn despawn_instance(&mut self, handle: VGInstanceHandle) {
        if let Some(slot) = self.instance_map.remove(&handle) {
            // Mark slot as free by zeroing the meshlet_count
            self.instance_data[slot].meshlet_count = 0;
            self.instances_dirty = true;
        }
    }

    fn find_free_instance_slot(&self) -> usize {
        // Find a slot whose meshlet_count == 0 (freed instance)
        for (i, inst) in self.instance_data.iter().enumerate() {
            if inst.meshlet_count == 0 && !self.instance_map.values().any(|&s| s == i) {
                return i;
            }
        }
        self.instance_data.len()
    }

    // ── Per-frame update ──────────────────────────────────────────────────────

    /// Prepare the frame: select LOD per instance, cull meshlets, upload draw commands.
    ///
    /// `view_proj` must be the same column-major matrix used by the camera buffer.
    /// `camera_pos` and `fov_y_rad` drive per-instance LOD selection via [`lod::select_lod`].
    pub fn prepare_frame(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: &[[f32; 4]; 4],
        camera_pos: redox_math::Vec3,
        fov_y_rad: f32,
    ) {
        // LOD selection per instance (stored in `flags` as selected level index).
        for inst in &mut self.instance_data {
            if inst.meshlet_count == 0 {
                continue;
            }
            let start = inst.meshlet_offset as usize;
            let _end = (inst.meshlet_offset + inst.meshlet_count) as usize;
            let first = self.global_meshlets.get(start).copied();
            if let Some(m) = first {
                let scale = super::culling::max_scale(&inst.transform);
                let world_center =
                    super::culling::transform_point(&inst.transform, m.sphere_center);
                let dist = redox_math::Vec3::from_array(world_center).distance(camera_pos);
                let proj = super::lod::projected_radius(m.sphere_radius * scale, dist, fov_y_rad);
                inst.flags = super::lod::select_lod(proj, &self.config.lod_config) as u32;
            }
        }
        self.instances_dirty = true;

        // Upload dirty instance data (LOD flags may have changed).
        if self.instances_dirty && !self.instance_data.is_empty() {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instance_data),
            );
            self.instances_dirty = false;
        }

        if self.instance_data.is_empty() || self.global_meshlets.is_empty() {
            self.indirect.count = 0;
            self.stats = VGStats {
                total_meshlets: self.global_meshlets.len() as u32,
                visible_meshlets: 0,
                total_triangles: self
                    .global_meshlets
                    .iter()
                    .map(|m| (m.index_count / 3) as u64)
                    .sum(),
                visible_triangles: 0,
                registered_assets: self.assets.len() as u32,
                active_instances: self
                    .instance_data
                    .iter()
                    .filter(|i| i.meshlet_count > 0)
                    .count() as u32,
            };
            return;
        }

        let frustum = Frustum::from_view_proj(view_proj);
        let cam = [camera_pos.x, camera_pos.y, camera_pos.z];

        if !self.config.gpu_cull {
            let result = cull_meshlets_into(
                &self.instance_data,
                &self.global_meshlets,
                &frustum,
                cam,
                &mut self.cull_commands_scratch,
            );
            let total_tris: u64 = self
                .global_meshlets
                .iter()
                .map(|m| (m.index_count / 3) as u64)
                .sum();

            self.indirect.upload(queue, &self.cull_commands_scratch);

            self.stats = VGStats {
                total_meshlets: result.total_meshlet_count,
                visible_meshlets: result.visible_meshlet_count,
                total_triangles: total_tris,
                visible_triangles: result.visible_triangle_count,
                registered_assets: self.assets.len() as u32,
                active_instances: self
                    .instance_data
                    .iter()
                    .filter(|i| i.meshlet_count > 0)
                    .count() as u32,
            };
        } else {
            // GPU cull runs in Renderer::render_frame_into; seed CPU cull once.
            if self.indirect.count == 0 {
                let result = cull_meshlets_into(
                    &self.instance_data,
                    &self.global_meshlets,
                    &frustum,
                    cam,
                    &mut self.cull_commands_scratch,
                );
                self.indirect.upload(queue, &self.cull_commands_scratch);
                self.stats.visible_meshlets = result.visible_meshlet_count;
            }
            // GPU cull runs in Renderer::render_frame_into before draw calls.
            self.stats = VGStats {
                total_meshlets: self.global_meshlets.len() as u32,
                visible_meshlets: self.indirect.count,
                total_triangles: self
                    .global_meshlets
                    .iter()
                    .map(|m| (m.index_count / 3) as u64)
                    .sum(),
                visible_triangles: 0,
                registered_assets: self.assets.len() as u32,
                active_instances: self
                    .instance_data
                    .iter()
                    .filter(|i| i.meshlet_count > 0)
                    .count() as u32,
            };
        }
    }

    /// Upload meshlet SSBO when assets changed.
    pub fn upload_meshlets_if_dirty(&mut self, queue: &wgpu::Queue) {
        if !self.meshlets_dirty {
            return;
        }
        if !self.global_meshlets.is_empty() {
            queue.write_buffer(
                &self.meshlet_gpu_buffer,
                0,
                bytemuck::cast_slice(&self.global_meshlets),
            );
        }
        self.meshlets_dirty = false;
    }

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
        hiz: &super::hiz::HiZPyramid,
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
        hiz: &super::hiz::HiZPyramid,
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
        hiz: &super::hiz::HiZPyramid,
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
        hiz: &super::hiz::HiZPyramid,
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

    /// Issue visibility-buffer indirect draws.
    pub fn draw_vis_indirect<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        vis: &'a crate::pass::vg_resolve::VgVisPass,
        frame_bg: &'a wgpu::BindGroup,
    ) {
        if self.indirect.count == 0 {
            return;
        }
        crate::pass::vg_resolve::bind_vis_draw_state(
            pass,
            vis,
            frame_bg,
            &self.instances_bind_group,
            &self.vertex_buffer,
            &self.index_buffer,
        );
        self.indirect.issue_draws(pass);
    }

    /// Issue shadow-map indirect draws.
    pub fn draw_shadow_indirect<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        shadow: &'a crate::pass::vg_resolve::VgShadowPass,
        shadow_matrix_bg: &'a wgpu::BindGroup,
        instances_bg: &'a wgpu::BindGroup,
    ) {
        shadow.draw(
            pass,
            shadow_matrix_bg,
            instances_bg,
            &self.vertex_buffer,
            &self.index_buffer,
            &self.indirect,
        );
    }

    pub fn instance_buffer(&self) -> &wgpu::Buffer {
        &self.instance_buffer
    }

    pub fn meshlet_buffer(&self) -> &wgpu::Buffer {
        &self.meshlet_gpu_buffer
    }

    pub fn active_instance_count(&self) -> u32 {
        self.instance_data
            .iter()
            .filter(|i| i.meshlet_count > 0)
            .count() as u32
    }

    // ── Rendering (debug forward path) ────────────────────────────────────────

    /// Issue draw calls for all visible meshlets (debug teal-normals path).
    ///
    /// Must be called inside an active `RenderPass` configured for the same
    /// surface format and depth format used during construction.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.indirect.count == 0 {
            return;
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, &self.instances_bind_group, &[]);

        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        self.indirect.issue_draws(pass);
    }

    /// Return statistics from the most recent `prepare_frame` call.
    pub fn stats(&self) -> &VGStats {
        &self.stats
    }

    /// Returns `true` if any instances are registered.
    pub fn has_content(&self) -> bool {
        self.indirect.count > 0
    }
}

// ── Pipeline builder ──────────────────────────────────────────────────────────

fn build_pipeline(
    device: &wgpu::Device,
    camera_bgl: &wgpu::BindGroupLayout,
    instances_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("vg_pipeline_layout"),
        bind_group_layouts: &[camera_bgl, instances_bgl],
        push_constant_ranges: &[],
    });

    let shader_src = crate::shader::shader_with_vertex_decode(crate::shader::VG_WGSL);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("vg_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("vg_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[CompressedVertex::buffer_layout()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    })
}

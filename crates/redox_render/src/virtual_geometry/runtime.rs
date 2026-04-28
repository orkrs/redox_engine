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

use std::collections::HashMap;
use bytemuck;

use crate::mesh::Vertex;
use super::asset_pipeline::build_vg_asset;
use super::culling::{cull_meshlets, Frustum};
use super::indirect::IndirectDrawBuffer;
use super::lod::LodChainConfig;
use super::meshlet::{
    MeshletDescriptor, VGAssetData, VGInstanceData,
    MAX_GLOBAL_MESHLETS, MAX_VG_INSTANCES, MAX_VISIBLE_COMMANDS,
};

// ── WGSL shader source ────────────────────────────────────────────────────────

/// Vertex + fragment shader for virtual geometry rendering.
///
/// Group 0: camera uniform (binding 0)
/// Group 1: per-instance storage buffer (binding 0)
///
/// The fragment stage renders world-space normals tinted with a VG debug colour
/// so it is easy to distinguish VG objects from standard PBR objects.
/// Full PBR integration (IBL, clustered lights, VSM shadows) can be added by
/// extending the bind groups.
const VG_SHADER_SRC: &str = r#"
// ── Shared structures ────────────────────────────────────────────────────

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct VGInstance {
    transform: mat4x4<f32>,
    meshlet_offset: u32,
    meshlet_count: u32,
    flags: u32,
    _pad: u32,
};

// ── Bindings ─────────────────────────────────────────────────────────────

@group(0) @binding(0) var<uniform> camera: CameraUniform;

@group(1) @binding(0) var<storage, read> vg_instances: array<VGInstance>;

// ── Vertex input / output ─────────────────────────────────────────────────

struct VertIn {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) uv:       vec2<f32>,
    @location(3) tangent:  vec3<f32>,
};

struct VertOut {
    @builtin(position) clip_pos:    vec4<f32>,
    @location(0)       world_pos:   vec3<f32>,
    @location(1)       world_norm:  vec3<f32>,
    @location(2)       uv:          vec2<f32>,
};

// ── Vertex shader ─────────────────────────────────────────────────────────

@vertex
fn vs_main(
    in: VertIn,
    @builtin(instance_index) inst_idx: u32,
) -> VertOut {
    let inst = vg_instances[inst_idx];
    let world_pos  = inst.transform * vec4<f32>(in.position, 1.0);
    let world_norm = normalize((inst.transform * vec4<f32>(in.normal, 0.0)).xyz);

    var out: VertOut;
    out.clip_pos   = camera.view_proj * world_pos;
    out.world_pos  = world_pos.xyz;
    out.world_norm = world_norm;
    out.uv         = in.uv;
    return out;
}

// ── Fragment shader ───────────────────────────────────────────────────────
// Renders world-space normals as colour with a teal tint, making VG objects
// visually distinct from PBR objects (useful for debugging + demos).

@fragment
fn fs_main(in: VertOut) -> @location(0) vec4<f32> {
    // Normal → colour (0.5*N + 0.5 remaps [-1,1] to [0,1])
    let n_col = in.world_norm * 0.5 + vec3<f32>(0.5);

    // Teal tint to distinguish VG objects
    let tint = vec3<f32>(0.2, 0.85, 0.9);
    let colour = mix(n_col, tint, 0.35);

    return vec4<f32>(colour, 1.0);
}
"#;

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
}

impl Default for VGConfig {
    fn default() -> Self {
        Self {
            max_vertices: 1 << 22,   // ~4M
            max_indices: 1 << 23,    // ~8M
            max_meshlets: MAX_GLOBAL_MESHLETS,
            max_instances: MAX_VG_INSTANCES,
            max_visible_commands: MAX_VISIBLE_COMMANDS,
            lod_config: LodChainConfig::default(),
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
    /// Offset of this asset's meshlets in the global meshlet buffer.
    meshlet_offset: u32,
    /// Number of meshlets for this asset.
    meshlet_count: u32,
    /// Offset in global vertex buffer (reserved for future streaming).
    vertex_offset: u32,
    /// Offset in global index buffer (reserved for future streaming).
    index_offset: u32,
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

    // Indirect draw
    indirect: IndirectDrawBuffer,

    // Pipeline
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
        let vb_size = (config.max_vertices * std::mem::size_of::<Vertex>()) as u64;
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

        // ── Indirect Draw Buffer ──────────────────────────────────────────────
        let indirect = IndirectDrawBuffer::new(device, config.max_visible_commands, "vg_indirect_draw");

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
            indirect,
            pipeline,
            camera_bgl,
            camera_bind_group,
            instances_bgl,
            instances_bind_group,
            global_meshlets: Vec::new(),
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
    pub fn register_asset(
        &mut self,
        data: VGAssetData,
        queue: &wgpu::Queue,
    ) -> Option<VGAssetId> {
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

        // Upload vertices
        queue.write_buffer(
            &self.vertex_buffer,
            (vert_offset as usize * std::mem::size_of::<Vertex>()) as u64,
            bytemuck::cast_slice(&data.vertices),
        );

        // Upload indices
        queue.write_buffer(
            &self.index_buffer,
            (idx_offset as usize * std::mem::size_of::<u32>()) as u64,
            bytemuck::cast_slice(&data.indices),
        );

        // Accumulate meshlets into CPU mirror (used every frame for culling)
        self.global_meshlets.extend(data.meshlets);

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
            },
        );

        log::info!(
            "[VGSystem] Registered VG asset {:?}: {} meshlets, {} verts, {} indices",
            id, meshlet_count, vert_needed, idx_needed
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
        let data = build_vg_asset(mesh, &self.config.lod_config.clone(), material_index);
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

    /// Prepare the frame: cull meshlets and upload draw commands.
    ///
    /// `view_proj` must be the same column-major matrix used by the camera buffer.
    pub fn prepare_frame(&mut self, queue: &wgpu::Queue, view_proj: &[[f32; 4]; 4]) {
        // Upload dirty instance data
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
                total_triangles: self.global_meshlets.iter().map(|m| (m.index_count / 3) as u64).sum(),
                visible_triangles: 0,
                registered_assets: self.assets.len() as u32,
                active_instances: self.instance_data.iter().filter(|i| i.meshlet_count > 0).count() as u32,
            };
            return;
        }

        let frustum = Frustum::from_view_proj(view_proj);
        let result = cull_meshlets(&self.instance_data, &self.global_meshlets, &frustum);

        let total_tris: u64 = self.global_meshlets.iter().map(|m| (m.index_count / 3) as u64).sum();

        self.indirect.upload(queue, &result.commands);

        self.stats = VGStats {
            total_meshlets: result.total_meshlet_count,
            visible_meshlets: result.visible_meshlet_count,
            total_triangles: total_tris,
            visible_triangles: result.visible_triangle_count,
            registered_assets: self.assets.len() as u32,
            active_instances: self.instance_data.iter().filter(|i| i.meshlet_count > 0).count() as u32,
        };
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    /// Issue draw calls for all visible meshlets.
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

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("vg_shader"),
        source: wgpu::ShaderSource::Wgsl(VG_SHADER_SRC.into()),
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("vg_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs_main",
            buffers: &[Vertex::buffer_layout()],
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

